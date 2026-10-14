//! Paired GPU opening + two-edit cache-policy benchmark. Parsing, device/compositor creation,
//! readback and CPU validation are outside timings; timed render phases include GPU completion.
#![forbid(unsafe_code)]
use photocraft_doc::Document;
use photocraft_geom::Rect;
use photocraft_gpu::Compositor;
use serde_json::{Value, json};
use std::{error::Error, time::Instant};

fn edit(doc: &mut Document, n: usize, damage: Rect) -> Result<(), Box<dyn Error>> {
    let layer = doc.layers.last_mut().ok_or("fixture has no top layer")?;
    let surface = layer.surface_mut().ok_or("fixture top layer must have raster pixels")?;
    surface.fill_rect(damage, &[0.25 + n as f32 * 0.2, 0.6, 0.1, 0.7]);
    Ok(())
}
fn wait(device: &wgpu::Device) -> Result<(), Box<dyn Error>> {
    device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None })?;
    Ok(())
}
fn measured_render(comp: &mut Compositor, device: &wgpu::Device, queue: &wgpu::Queue, doc: &Document, rect: Rect) -> Result<Value, Box<dyn Error>> {
    let start = Instant::now();
    let stats = comp.render(device, queue, doc, rect, |_, _| {})?;
    wait(device)?;
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    Ok(json!({"ms":ms,"prefix_hits":stats.prefix_hits,"prefix_passes_skipped":stats.prefix_passes_skipped,
        "cache_bytes":comp.composite_cache_bytes(),"passes":stats.passes,"uploads":stats.tiles_uploaded,
        "bytes_uploaded":stats.bytes_uploaded}))
}
fn validate(comp: &mut Compositor, device: &wgpu::Device, queue: &wgpu::Queue, doc: &Document, rect: Rect) -> Result<f32, Box<dyn Error>> {
    let cpu = photocraft_compose::render(doc, rect);
    let gpu = photocraft_gpu::render_to_vec(comp, device, queue, doc, rect)?;
    if cpu.px.len() != gpu.len() {
        return Err("CPU/GPU lengths differ".into());
    }
    let mut error = 0.0f32;
    for (a, b) in cpu.px.iter().zip(&gpu) {
        for (a, b) in a.iter().zip(b) {
            if !a.is_finite() || !b.is_finite() {
                return Err("nonfinite composite".into());
            }
            error = error.max((a - b).abs());
        }
    }
    if error > 1.0 / 255.0 + 1e-5 {
        return Err(format!("CPU/GPU error {error}").into());
    }
    Ok(error)
}
fn run() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let fixture = args.first().ok_or("usage: psd-cache-policy FIXTURE OUTPUT [paired repetitions=3]")?;
    let output = args.get(1).ok_or("missing output path")?;
    let reps: usize = args.get(2).map_or(Ok(3), |s| s.parse())?;
    if !(1..=10).contains(&reps) {
        return Err("repetitions must be 1..10".into());
    }
    let bytes = std::fs::read(fixture)?;
    let imported = photocraft_io::import(fixture, &bytes)?;
    if !imported.warnings.is_empty() {
        return Err(format!("import warnings: {:?}", imported.warnings).into());
    }
    let source = imported.document;
    let damage = Rect::new(128, 128, 192, 192).intersect(&source.bounds());
    if damage.is_empty() {
        return Err("fixture must cover the edit rectangle".into());
    }
    let instance = wgpu::Instance::default();
    let adapter = pollster::block_on(
        instance.request_adapter(&wgpu::RequestAdapterOptions { power_preference: wgpu::PowerPreference::HighPerformance, ..Default::default() }),
    )?;
    let limits = adapter.limits();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        required_limits: wgpu::Limits {
            max_texture_dimension_2d: limits.max_texture_dimension_2d,
            max_buffer_size: limits.max_buffer_size,
            ..Default::default()
        },
        ..Default::default()
    }))?;
    let format = Compositor::preferred_acc_format(&adapter);
    let budget = 512 << 20;
    let mut samples = Vec::new();
    let mut cold_pair = Vec::new();
    for repetition in 0..=reps {
        // Alternate paired policy order to reduce drift; every sample owns a fresh compositor.
        for deferred in if repetition % 2 == 0 { [false, true] } else { [true, false] } {
            let mut doc = source.clone();
            let mut comp = Compositor::try_new_with_format(&device, format)?;
            comp.set_composite_cache_budget(budget);
            if deferred {
                comp.defer_composite_cache_population();
            }
            wait(&device)?;
            let open = measured_render(&mut comp, &device, &queue, &doc, doc.bounds())?;
            let mut edits = Vec::new();
            for n in 0..2 {
                edit(&mut doc, n, damage)?;
                edits.push(measured_render(&mut comp, &device, &queue, &doc, damage)?);
            }
            let total = open["ms"].as_f64().ok_or("invalid open time")? + edits.iter().filter_map(|v| v["ms"].as_f64()).sum::<f64>();
            // Readback after ALL timed phases cannot warm a cache for subsequent timings.
            let error = validate(&mut comp, &device, &queue, &doc, damage)?;
            let sample = json!({"repetition":repetition,"deferred":deferred,"open":open,"edits":edits,"total_ms":total,"final_edit_cpu_error":error});
            println!("{sample}");
            if repetition == 0 {
                cold_pair.push(sample);
            } else {
                samples.push(sample);
            }
        }
    }
    // Separate untimed replay checks representative opening crops and both edits against
    // the CPU oracle under both policies; none of these readbacks enter timing samples.
    let mut validation = Vec::new();
    for deferred in [false, true] {
        let mut doc = source.clone();
        let mut comp = Compositor::try_new_with_format(&device, format)?;
        comp.set_composite_cache_budget(budget);
        if deferred {
            comp.defer_composite_cache_population();
        }
        comp.render(&device, &queue, &doc, doc.bounds(), |_, _| {})?;
        wait(&device)?;
        let bounds = doc.bounds();
        for (label, rect) in [
            ("open-top-left", Rect::from_xywh(0, 0, 640, 480)),
            ("open-center", Rect::from_xywh(bounds.x1 / 2 - 320, bounds.y1 / 2 - 240, 640, 480)),
            ("open-bottom-right", Rect::from_xywh(bounds.x1 - 640, bounds.y1 - 480, 640, 480)),
        ] {
            let rect = rect.intersect(&bounds);
            let error = validate(&mut comp, &device, &queue, &doc, rect)?;
            validation.push(json!({"deferred":deferred,"phase":label,"cpu_error":error}));
        }
        for n in 0..2 {
            edit(&mut doc, n, damage)?;
            let error = validate(&mut comp, &device, &queue, &doc, damage)?;
            validation.push(json!({"deferred":deferred,"edit":n+1,"cpu_error":error}));
        }
    }
    if let Some(parent) = std::path::Path::new(output).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        output,
        serde_json::to_vec_pretty(&json!({
            "fixture":fixture,"fixture_blake3":blake3::hash(&bytes).to_hex().to_string(),
            "adapter":format!("{:?}",adapter.get_info()),"accumulator":format!("{format:?}"),
            "cache_budget":budget,"timing":"GPU render plus completion; import, edits, construction and validation excluded",
            "damage":[damage.x0,damage.y0,damage.x1,damage.y1],"cold_pair":cold_pair,"warmup":"one opening-plus-two-edits pair per policy excluded from samples","samples":samples,"validation":validation
        }))?,
    )?;
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
