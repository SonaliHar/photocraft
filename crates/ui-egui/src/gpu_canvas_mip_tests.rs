//! Read every mip, rather than only level zero: a clean 100% view does not prove that
//! reduced previews are valid. Keep all cases on one device (some drivers dislike concurrent
//! test devices), and use synthetic pixels instead of event photographs.

use super::*;
use photocraft_compose::Buffer;
use photocraft_doc::SampleType;
use photocraft_geom::Rect;

fn read_mips(gpu: &GpuCanvas, key: u64) -> Vec<Vec<[f32; 4]>> {
    ensure_test_mips(gpu, key, usize::MAX);
    read_current_mips(gpu, key)
}

fn ensure_test_mips(gpu: &GpuCanvas, key: u64, max_level: usize) {
    let renderer = gpu.rs.renderer.read();
    let res = renderer.callback_resources.get::<Resources>().unwrap();
    res.docs[&key].ensure_mips(&gpu.rs.device, &gpu.rs.queue, res, max_level, |_| true);
}

fn read_current_mips(gpu: &GpuCanvas, key: u64) -> Vec<Vec<[f32; 4]>> {
    let renderer = gpu.rs.renderer.read();
    let doc = &renderer.callback_resources.get::<Resources>().unwrap().docs[&key];
    assert_eq!(doc.tiles.len(), 1);
    let tile = &doc.tiles[0];
    let (device, queue) = (&gpu.rs.device, &gpu.rs.queue);
    let bpp = texel_bytes(doc.format) as usize;
    (0..tile.levels.len())
        .map(|level| {
            let (w, h) = ((doc.size[0] >> level).max(1), (doc.size[1] >> level).max(1));
            let row = (w as usize * bpp).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT as usize);
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("mip_test_readback"),
                size: (row * h as usize) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&Default::default());
            encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo { texture: &tile.texture, mip_level: level as u32, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
                wgpu::TexelCopyBufferInfo {
                    buffer: &buffer,
                    layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row as u32), rows_per_image: Some(h) },
                },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            queue.submit([encoder.finish()]);
            buffer.slice(..).map_async(wgpu::MapMode::Read, |result| result.unwrap());
            assert!(gpu.health.wait(device, None));
            let data = buffer.slice(..).get_mapped_range().unwrap();
            let mut pixels = Vec::new();
            for y in 0..h as usize {
                for x in 0..w as usize {
                    let offset = y * row + x * bpp;
                    pixels.push(texel_to_f32(doc.format, &data[offset..offset + bpp]));
                }
            }
            pixels
        })
        .collect()
}

#[test]
fn canvas_mips_preserve_opaque_pixels_and_partial_updates() {
    let rs = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| egui_kittest::wgpu::create_render_state(wgpu_setup(), Default::default())));
    let Ok(rs) = rs else {
        eprintln!("skipping mip readback: no GPU adapter");
        return;
    };
    eprintln!("mip readback adapter: {:?}", rs.adapter.get_info());
    let gpu = GpuCanvas::new(&rs);
    // Both mip paths: straight into the level, and the DX12 scratch-texture workaround.
    for separate in [false, true] {
        gpu.rs.renderer.write().callback_resources.get_mut::<Resources>().unwrap().separate_mip_targets = separate;
        check_mips(&gpu);
        check_demand_mips(&gpu);
    }
}

fn check_mips(gpu: &GpuCanvas) {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for (w, h) in [(1791, 1083), (513, 1), (1, 513)] {
            let mut full = Buffer::filled(Rect::from_xywh(0, 0, w, h), [0.25, 0.5, 0.75, 1.0]);
            // Reusing the key also covers replacing textures after a size or depth change.
            gpu.upload_buffer_full(1, &full, depth);
            for (level, pixels) in read_mips(gpu, 1).iter().enumerate() {
                for pixel in pixels {
                    assert!((pixel[3] - 1.0).abs() < 0.001, "{depth:?} {w}x{h} mip {level}: {pixel:?}");
                    for (actual, expected) in pixel.iter().zip([0.25, 0.5, 0.75, 1.0]) {
                        assert!((actual - expected).abs() < 0.005, "{depth:?} {w}x{h} mip {level}: {pixel:?}");
                    }
                }
            }
            let patch = Rect::from_xywh((w / 2) as i32, (h / 2) as i32, (w / 3).max(1), (h / 3).max(1));
            let changed = Buffer::filled(patch, [1.0, 0.0, 0.0, 1.0]);
            assert!(gpu.upload_buffer_rect(1, &changed));
            for y in patch.y0..patch.y1 {
                for x in patch.x0..patch.x1 {
                    full.px[y as usize * w as usize + x as usize] = [1.0, 0.0, 0.0, 1.0];
                }
            }
            gpu.upload_buffer_full(2, &full, depth);
            let partial = read_mips(gpu, 1);
            let rebuilt = read_mips(gpu, 2);
            for (level, (actual, expected)) in partial.iter().zip(&rebuilt).enumerate() {
                for (a, b) in actual.iter().zip(expected) {
                    assert!((a[3] - 1.0).abs() < 0.001, "partial {depth:?} mip {level}: {a:?}");
                    for (a, b) in a.iter().zip(b) {
                        assert!((a - b).abs() < 0.005, "partial/full mismatch {depth:?} {w}x{h} mip {level}: {a} != {b}");
                    }
                }
            }
        }
    }
}

/// Canvas-only opening metric: PSD decoding, texture allocation, source generation and
/// correctness readback are outside the timer. Run the same test binary built from each
/// revision; the first sample of each scenario warms pipelines and is recorded separately.
#[test]
#[ignore = "36 MP GPU-wait-inclusive canvas performance experiment"]
fn canvas_open_36mp_performance() {
    let rs = egui_kittest::wgpu::create_render_state(wgpu_setup(), Default::default());
    let gpu = GpuCanvas::new(&rs);
    let size = [7360u32, 4912u32];
    let mut rgba = Vec::with_capacity(size[0] as usize * size[1] as usize * 4);
    for y in 0..size[1] {
        for x in 0..size[0] {
            rgba.extend_from_slice(&benchmark_pixel(x, y));
        }
    }
    let patch_origin = [size[0] / 2, size[1] / 2];
    let patch = [213u8, 47, 101, 255].repeat(64 * 64);
    let rounds = std::env::var("PHOTOCRAFT_CANVAS_BENCH_ROUNDS").ok().and_then(|s| s.parse::<usize>().ok()).unwrap_or(6).clamp(1, 32);
    let mut samples = Vec::new();
    for (scenario, zoom, navigator) in [("one-to-one", 1.0, false), ("fit", 0.125, false), ("fit-and-navigator", 0.125, true)] {
        for round in 0..=rounds {
            let key = 9100;
            {
                let mut renderer = gpu.rs.renderer.write();
                let res = renderer.callback_resources.get_mut::<Resources>().unwrap();
                let textures = DocTextures::new(&gpu.rs.device, res, size, gpu.tile, FORMAT).unwrap();
                res.docs.insert(key, textures);
            }
            assert!(gpu.health.wait(&gpu.rs.device, None));
            let start = std::time::Instant::now();
            gpu.upload_full(key, size, &rgba);
            benchmark_prepare_views(&gpu, key, size, zoom, navigator);
            assert!(gpu.health.wait(&gpu.rs.device, None));
            let opening_ms = start.elapsed().as_secs_f64() * 1000.0;
            let mut edits_ms = Vec::new();
            for dx in [0, 96] {
                let start = std::time::Instant::now();
                assert!(gpu.upload_rect(key, [patch_origin[0] + dx, patch_origin[1]], [64, 64], &patch));
                benchmark_prepare_views(&gpu, key, size, zoom, navigator);
                assert!(gpu.health.wait(&gpu.rs.device, None));
                edits_ms.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            // Readbacks stay outside timing; they prove upload and both edit locations.
            for (point, expected) in [
                ([0, 0], benchmark_pixel(0, 0)),
                ([size[0] - 1, size[1] - 1], benchmark_pixel(size[0] - 1, size[1] - 1)),
                (patch_origin, [213, 47, 101, 255]),
                ([patch_origin[0] + 96, patch_origin[1]], [213, 47, 101, 255]),
            ] {
                assert_eq!(benchmark_read_pixel(&gpu, key, point), expected, "{scenario} round {round} at {point:?}");
            }
            samples.push(serde_json::json!({
                "scenario": scenario, "round": round, "warmup": round == 0,
                "opening_ms": opening_ms, "edits_ms": edits_ms,
                "opening_and_two_edits_ms": opening_ms + edits_ms.iter().sum::<f64>(),
            }));
        }
    }
    let result = serde_json::json!({
        "metric": "canvas upload and required view mip work, including GPU wait",
        "size": size, "tile": gpu.tile, "format": "Rgba8Unorm",
        "adapter": format!("{:?}", rs.adapter.get_info()),
        "excluded": ["PSD decoding", "texture allocation", "source generation", "readback", "painting the canvas to the screen"],
        "validation": "exact level-zero bytes at canvas corners and both edit origins; full mip fidelity has separate tests",
        "samples": samples,
    });
    let json = serde_json::to_string_pretty(&result).unwrap();
    println!("CANVAS_OPEN_36MP_JSON\n{json}");
    if let Ok(path) = std::env::var("PHOTOCRAFT_CANVAS_BENCH_OUTPUT") {
        std::fs::write(path, json).unwrap();
    }
}

fn benchmark_pixel(x: u32, y: u32) -> [u8; 4] {
    if (x / 32 + y / 32).is_multiple_of(2) { [37, 89, 173, 255] } else { [191, 127, 53, 255] }
}

fn benchmark_prepare_views(gpu: &GpuCanvas, key: u64, size: [u32; 2], zoom: f32, navigator: bool) {
    let screen = ScreenDescriptor { size_in_pixels: [1024, 768], pixels_per_point: 1.0 };
    let mut renderer = gpu.rs.renderer.write();
    let mut encoder = gpu.rs.device.create_command_encoder(&Default::default());
    for (view_key, zoom, rect) in std::iter::once((1, zoom, egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 768.0))))
        .chain(navigator.then_some((2, 0.025, egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(184.0, 124.0)))))
    {
        let callback = CanvasCallback {
            rect,
            params: ViewParams {
                doc: key,
                doc_size: size,
                zoom,
                center: [size[0] as f32 / 2.0, size[1] as f32 / 2.0],
                rotation: 0.0,
                shadow: false,
                pixel_grid: false,
                view_key,
                display: 0,
                output: 0,
                hdr: None,
                offset: [0.0, 0.0],
            },
        };
        let commands = callback.prepare(&gpu.rs.device, &gpu.rs.queue, &screen, &mut encoder, &mut renderer.callback_resources);
        gpu.rs.queue.submit(commands);
    }
    gpu.rs.queue.submit([encoder.finish()]);
}

fn benchmark_read_pixel(gpu: &GpuCanvas, key: u64, point: [u32; 2]) -> [u8; 4] {
    let renderer = gpu.rs.renderer.read();
    let doc = &renderer.callback_resources.get::<Resources>().unwrap().docs[&key];
    let tile = doc
        .tiles
        .iter()
        .find(|t| point[0] >= t.rect[0] && point[1] >= t.rect[1] && point[0] < t.rect[0] + t.rect[2] && point[1] < t.rect[1] + t.rect[3])
        .unwrap();
    let row = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.rs.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("canvas_benchmark_readback"),
        size: u64::from(row),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.rs.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &tile.texture,
            mip_level: 0,
            origin: wgpu::Origin3d { x: point[0] - tile.rect[0], y: point[1] - tile.rect[1], z: 0 },
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo { buffer: &buffer, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(1) } },
        wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
    );
    gpu.rs.queue.submit([encoder.finish()]);
    buffer.slice(..).map_async(wgpu::MapMode::Read, |r| r.unwrap());
    assert!(gpu.health.wait(&gpu.rs.device, None));
    let data = buffer.slice(..).get_mapped_range().unwrap();
    data[..4].try_into().unwrap()
}

fn check_demand_mips(gpu: &GpuCanvas) {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for (w, h) in [(513, 257), (513, 1), (1, 513)] {
            let rect = Rect::from_xywh(0, 0, w, h);
            let mut full = Buffer::filled(rect, [0.0, 0.0, 0.0, 1.0]);
            for y in 0..h {
                for x in 0..w {
                    let p = benchmark_pixel(x, y);
                    full.px[(y * w + x) as usize] = p.map(|v| f32::from(v) / 255.0);
                }
            }
            gpu.upload_buffer_full(11, &full, depth);
            gpu.upload_buffer_full(12, &full, depth);
            let reference = read_mips(gpu, 12);
            ensure_test_mips(gpu, 11, 0);
            assert_mip_damage(gpu, 11, 0);
            // Fractional LOD 1.5 samples both levels 1 and 2.
            let max_level = filter_mode(2.0f32.powf(-1.5)).1.ceil() as usize;
            assert_eq!(max_level, 2);
            ensure_test_mips(gpu, 11, max_level);
            assert_mip_damage(gpu, 11, 2);
            let actual = read_current_mips(gpu, 11);
            compare_mip_prefix(&actual, &reference, 2, depth);
            let patch = Rect::from_xywh((w / 2) as i32, (h / 2) as i32, 1, 1);
            let changed = Buffer::filled(patch, [1.0, 0.0, 0.0, 1.0]);
            assert!(gpu.upload_buffer_rect(11, &changed));
            full.px[(h / 2 * w + w / 2) as usize] = [1.0, 0.0, 0.0, 1.0];
            gpu.upload_buffer_full(12, &full, depth);
            let reference = read_mips(gpu, 12);
            ensure_test_mips(gpu, 11, 1);
            assert_mip_damage(gpu, 11, 1);
            compare_mip_prefix(&read_current_mips(gpu, 11), &reference, 1, depth);
            // A disjoint second edit must join the pending damage in deeper levels after the
            // first edit's shallow prefix was already generated.
            let second_patch = Rect::from_xywh((w - 1) as i32, (h - 1) as i32, 1, 1);
            assert!(gpu.upload_buffer_rect(11, &Buffer::filled(second_patch, [0.0, 1.0, 0.0, 1.0])));
            full.px[(h * w - 1) as usize] = [0.0, 1.0, 0.0, 1.0];
            ensure_test_mips(gpu, 11, 1);
            assert_mip_damage(gpu, 11, 1);
            gpu.upload_buffer_full(12, &full, depth);
            let reference = read_mips(gpu, 12);
            compare_mip_prefix(&read_current_mips(gpu, 11), &reference, 1, depth);
            // A later zoom-out needs both pending edits even after the closer edit views.
            compare_mip_prefix(&read_mips(gpu, 11), &reference, usize::MAX, depth);
            assert_mip_damage(gpu, 11, usize::MAX);
        }
    }
    // Tile visibility and a later pan: the first request must leave the other tile dirty.
    let tiled = GpuCanvas { tile: 64, ..gpu.clone() };
    let full = Buffer::filled(Rect::from_xywh(0, 0, 129, 65), [0.25, 0.5, 0.75, 1.0]);
    tiled.upload_buffer_full(21, &full, SampleType::U8);
    {
        let renderer = tiled.rs.renderer.read();
        let res = renderer.callback_resources.get::<Resources>().unwrap();
        let doc = &res.docs[&21];
        doc.ensure_mips(&tiled.rs.device, &tiled.rs.queue, res, 2, |t| t.rect[0] == 0);
        for tile in &doc.tiles {
            let damage = tile.mip_damage.lock().unwrap();
            for pending in damage.iter().skip(1).take(2) {
                assert_eq!(pending.is_none(), tile.rect[0] == 0);
            }
        }
        doc.ensure_mips(&tiled.rs.device, &tiled.rs.queue, res, 2, |t| t.rect[0] != 0);
        for tile in &doc.tiles {
            assert!(tile.mip_damage.lock().unwrap().iter().skip(1).take(2).all(Option::is_none));
        }
    }
    check_rotated_offset_pan(gpu);
    // View callbacks independently demand their LOD: a 1x main view skips mips, a Navigator
    // callback at the same revision catches them up, and a second unchanged callback is a no-op.
    gpu.upload_buffer_full(11, &Buffer::filled(Rect::from_xywh(0, 0, 1, 513), [0.25, 0.5, 0.75, 1.0]), SampleType::U8);
    benchmark_prepare_views(gpu, 11, [1, 513], 1.0, false);
    assert_mip_damage(gpu, 11, 0);
    benchmark_prepare_views(gpu, 11, [1, 513], 1.0, true);
    let navigator_level = filter_mode(0.025).1.ceil() as usize;
    assert_mip_damage(gpu, 11, navigator_level);
    benchmark_prepare_views(gpu, 11, [1, 513], 1.0, true);
    assert_mip_damage(gpu, 11, navigator_level);
    benchmark_prepare_views(gpu, 11, [1, 513], 0.001, false);
    assert_mip_damage(gpu, 11, usize::MAX);
}

fn assert_mip_damage(gpu: &GpuCanvas, key: u64, generated_through: usize) {
    let renderer = gpu.rs.renderer.read();
    let doc = &renderer.callback_resources.get::<Resources>().unwrap().docs[&key];
    for tile in &doc.tiles {
        for (level, damage) in tile.mip_damage.lock().unwrap().iter().enumerate().skip(1) {
            assert_eq!(damage.is_none(), level <= generated_through, "level {level}");
        }
    }
}

fn compare_mip_prefix(actual: &[Vec<[f32; 4]>], expected: &[Vec<[f32; 4]>], max_level: usize, depth: SampleType) {
    for (level, (actual, expected)) in actual.iter().zip(expected).enumerate().take(max_level.saturating_add(1)) {
        for (a, b) in actual.iter().zip(expected) {
            for (a, b) in a.iter().zip(b) {
                assert!((a - b).abs() < 0.005, "demand/eager mismatch {depth:?} mip {level}: {a} != {b}");
            }
        }
    }
}

fn check_rotated_offset_pan(gpu: &GpuCanvas) {
    let tiled = GpuCanvas { tile: 256, ..gpu.clone() };
    let size = [1299, 1025];
    tiled.upload_buffer_full(31, &Buffer::filled(Rect::from_xywh(0, 0, size[0], size[1]), [0.25, 0.5, 0.75, 1.0]), SampleType::U8);
    // A 128x96 view at .75 scale fits strictly inside a 256px tile, even when rotated 90deg.
    // Moving the whole Pattern Preview copy right by one document width must still demand
    // the underlying tile. A subsequent pan reaches the next tile with no upload/revision.
    for (pan, center_x) in [(false, 128.0), (true, 384.0)] {
        let callback = CanvasCallback {
            rect: egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(128.0, 96.0)),
            params: ViewParams {
                doc: 31,
                doc_size: size,
                zoom: 0.75,
                center: [size[0] as f32 + center_x, 128.0],
                rotation: std::f32::consts::FRAC_PI_2,
                shadow: false,
                pixel_grid: false,
                view_key: 31,
                display: 0,
                output: 0,
                hdr: None,
                offset: [size[0] as f32, 0.0],
            },
        };
        let screen = ScreenDescriptor { size_in_pixels: [1024, 768], pixels_per_point: 1.0 };
        let mut renderer = tiled.rs.renderer.write();
        let mut encoder = tiled.rs.device.create_command_encoder(&Default::default());
        let commands = callback.prepare(&tiled.rs.device, &tiled.rs.queue, &screen, &mut encoder, &mut renderer.callback_resources);
        tiled.rs.queue.submit(commands);
        tiled.rs.queue.submit([encoder.finish()]);
        let res = renderer.callback_resources.get::<Resources>().unwrap();
        for tile in &res.docs[&31].tiles {
            let visible_now_or_before = tile.rect[1] == 0 && (tile.rect[0] == 0 || (pan && tile.rect[0] == 256));
            let damage = tile.mip_damage.lock().unwrap();
            assert_eq!(damage[1].is_none(), visible_now_or_before, "rotated offset pan={pan}, tile {:?}", tile.rect);
            assert!(damage.iter().skip(2).all(Option::is_some), "unused deeper levels should remain pending");
        }
    }
}
