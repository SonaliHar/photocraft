//! Native wide export regressions and bounded extraction refusals.
use photocraft_engine::{
    EngineError, Session,
    doc::{ColorMode, Document, Layer, Rect, SampleType, Size},
    file_cmds::open_bytes_as,
};
use serde_json::json;
use std::io::Error;
fn native_case(sample: SampleType, name: &str, raw: Vec<u8>) -> Result<(), Box<dyn std::error::Error>> {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_nanos();
    let parent = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol1-layer-export-audit-20261012");
    std::fs::create_dir_all(&parent)?;
    let root = parent.join(format!("native-red-{name}-{}-{nanos}", std::process::id()));
    std::fs::create_dir(&root)?;
    let mut doc = Document::new("Own ordinary native export control", Size::new(3, 3), ColorMode::Rgb, sample);
    let resolution = photocraft_engine::doc::PhysicalResolution {
        x_dpi: 144.25,
        y_dpi: 287.75,
        unit: photocraft_engine::doc::PhysicalUnit::Custom(photocraft_engine::doc::CustomUnit {
            name: "Original test centimeter".into(),
            factor: 2.54,
            digits: 3,
            symbol: "cm".into(),
            abbreviation: "cm".into(),
        }),
    };
    doc.set_physical_resolution(resolution.clone())?;
    let mut layer = Layer::raster("Own opaque native pixel", doc.pixel_format());
    let id = layer.id;
    let source_area = Rect::new(1, 1, 2, 2);
    layer.surface_mut().ok_or_else(|| Error::other("missing source surface"))?.try_write_interleaved(source_area, &raw)?;
    doc.layers.push(layer);
    let mut session = Session::new();
    session.add_document(doc, None)?;
    session.edit("Own redo control", |doc, _| {
        doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?.name = "Own future name".into();
        Ok(())
    })?;
    if !session.undo() {
        return Err(Error::other("redo control setup failed").into());
    }
    let state = session.active().ok_or_else(|| Error::other("source absent"))?;
    let before = state.doc.clone();
    let initial = (state.revision, state.history.past_len(), state.history.can_redo(), state.active_layer);
    let path = root.join("own.pcraft");
    session.execute("layer.exportAs", json!({"layer":id.0,"path":path.to_string_lossy(),"scale":100}))?;
    let state = session.active().ok_or_else(|| Error::other("source disappeared"))?;
    if !state.doc.shares_state(&before) || (state.revision, state.history.past_len(), state.history.can_redo(), state.active_layer) != initial {
        return Err(Error::other("export mutated source or history").into());
    }
    let still = state.doc.layer(id).and_then(Layer::surface).ok_or_else(|| Error::other("source surface absent"))?.try_read_interleaved(source_area)?;
    if still != raw {
        return Err(Error::other("export mutated original native bytes").into());
    }
    let saved = std::fs::read(&path)?;
    let mut reopened = Session::new();
    open_bytes_as(&mut reopened, "own.pcraft", &saved, None, None)?;
    let current = &reopened.active().ok_or_else(|| Error::other("reopen absent"))?.doc;
    let native =
        current.layers.first().and_then(Layer::surface).ok_or_else(|| Error::other("reopened raster absent"))?.try_read_interleaved(current.bounds())?;
    assert_eq!((current.size.width, current.size.height), (1, 1));
    assert_eq!(current.effective_resolution(), resolution);
    assert_eq!(native, raw, "registered {name} native export lost source low bits");
    Ok(())
}
#[test]
fn registered_u32_export_keeps_opaque_native_low_bits() -> Result<(), Box<dyn std::error::Error>> {
    native_case(SampleType::U32, "u32", [0x80000001u32, 0x40000003, 0x10000007, u32::MAX].into_iter().flat_map(u32::to_ne_bytes).collect())
}
#[test]
fn registered_f64_export_keeps_finite_hdr_native_low_bits() -> Result<(), Box<dyn std::error::Error>> {
    native_case(
        SampleType::F64,
        "f64",
        [0.5 + 2f64.powi(-40), 1.25 + 2f64.powi(-38), -0.25 - 2f64.powi(-42), 1.0].into_iter().flat_map(f64::to_ne_bytes).collect(),
    )
}

fn control() -> (Document, photocraft_engine::doc::LayerId) {
    let mut doc = Document::new("Wide control", Size::new(520, 4), ColorMode::Rgb, SampleType::F64);
    let layer = Layer::raster("Wide raster", doc.pixel_format());
    let id = layer.id;
    doc.layers.push(layer);
    (doc, id)
}

#[test]
fn native_tile_crossing_crop_and_metadata() -> Result<(), Box<dyn std::error::Error>> {
    let (mut doc, id) = control();
    doc.resolution_dpi = 144.;
    let raw: Vec<u8> = [1.25 + 2f64.powi(-38), -0.25, 0.5, 1.].into_iter().flat_map(f64::to_ne_bytes).cycle().take(32 * 4).collect();
    doc.layer_mut(id).unwrap().surface_mut().unwrap().try_write_interleaved(Rect::new(254, 1, 258, 2), &raw)?;
    let out = photocraft_engine::layer_menu_cmds::layer_document(&doc, id)?;
    assert_eq!(out.size, Size::new(4, 1));
    assert_eq!(out.resolution_dpi, 144.);
    doc.layer_mut(id).unwrap().effects.enabled = false;
    let inactive_empty = photocraft_engine::layer_menu_cmds::layer_document(&doc, id)?;
    assert_eq!(inactive_empty.layers[0].surface().unwrap().try_read_interleaved(inactive_empty.bounds())?, raw);
    assert_eq!(out.layers[0].surface().unwrap().try_read_interleaved(out.bounds())?, raw);
    Ok(())
}

#[test]
fn canvas_clipping_and_hostile_tile_origins() -> Result<(), Box<dyn std::error::Error>> {
    let (mut doc, id) = control();
    let raw: Vec<u8> = [0.5, 0.25, -2., 1.].into_iter().flat_map(f64::to_ne_bytes).collect();
    let surface = doc.layer_mut(id).unwrap().surface_mut().unwrap();
    surface.try_write_interleaved(Rect::new(-1, 0, 0, 1), &raw)?;
    surface.try_write_interleaved(Rect::new(519, 3, 520, 4), &raw)?;
    let out = photocraft_engine::layer_menu_cmds::layer_document(&doc, id)?;
    assert_eq!(out.size, Size::new(1, 1));
    assert_eq!(out.layers[0].surface().unwrap().try_read_interleaved(out.bounds())?, raw);
    doc.layer_mut(id).unwrap().surface_mut().unwrap().tile_mut(photocraft_geom::TileCoord::new(i32::MAX / 256, 0));
    assert!(photocraft_engine::layer_menu_cmds::layer_document(&doc, id).is_err());
    Ok(())
}

#[test]
fn neutral_f32_behavior_is_retained() -> Result<(), Box<dyn std::error::Error>> {
    let mut doc = Document::new("Existing F32", Size::new(3, 3), ColorMode::Rgb, SampleType::F32);
    let mut layer = Layer::raster("Raster", doc.pixel_format());
    let id = layer.id;
    let raw: Vec<u8> = [0.5f32, 0.25, 0.75, 1.].into_iter().flat_map(f32::to_ne_bytes).collect();
    layer.surface_mut().unwrap().try_write_interleaved(Rect::new(1, 1, 2, 2), &raw)?;
    doc.layers.push(layer);
    let out = photocraft_engine::layer_menu_cmds::layer_document(&doc, id)?;
    assert_eq!(out.layers[0].surface().unwrap().try_read_interleaved(out.bounds())?, raw);
    Ok(())
}

#[test]
fn unsupported_wide_export_preserves_file_and_redo() -> Result<(), Box<dyn std::error::Error>> {
    for kind in 0..12 {
        let (mut doc, id) = control();
        let pixel = match kind {
            0 => [1., 0., 0., 0.5],
            1 => [1., 0., 0., 0.],
            2 => [f64::NAN, 0., 0., 1.],
            _ => [1., 0., 0., 1.],
        };
        let raw: Vec<u8> = pixel.into_iter().flat_map(f64::to_ne_bytes).collect();
        doc.layer_mut(id).unwrap().surface_mut().unwrap().try_write_interleaved(Rect::new(0, 0, 1, 1), &raw)?;
        match kind {
            3 => doc.layer_mut(id).unwrap().opacity = 0.5,
            4 => doc.layer_mut(id).unwrap().psd_blocks.push((*b"TySh", std::sync::Arc::new(vec![1, 2]))),
            5 => doc.icc_profile = Some(std::sync::Arc::new(vec![1, 2])),
            6 => doc.layer_mut(id).unwrap().fill_opacity = 0.5,
            7 => doc.layer_mut(id).unwrap().excluded_channels = 1,
            8 => doc.layer_mut(id).unwrap().mask = Some(photocraft_engine::doc::LayerMask::reveal_all()),
            9 => doc.sample_encoding = Some(photocraft_engine::doc::SampleEncoding::Linear),
            10 => doc.layer_mut(id).unwrap().effects.psd_raw = Some(std::sync::Arc::new(vec![1, 2])),
            11 => {
                let effects = &mut doc.layer_mut(id).unwrap().effects;
                effects.enabled = false;
                effects.items.push(photocraft_engine::doc::Effect::ColorOverlay {
                    common: photocraft_engine::doc::FxCommon::new(photocraft_color::BlendMode::Normal, 1.),
                    color: photocraft_color::Color::BLACK,
                });
            }
            _ => {}
        }
        let mut session = Session::new();
        session.add_document(doc, None)?;
        session.edit("Redo control", |d, _| {
            d.name = "Future".into();
            Ok(())
        })?;
        assert!(session.undo());
        let before = session.active().unwrap().doc.clone();
        let revision = session.active().unwrap().revision;
        let path = std::env::temp_dir().join(format!("photocraft-wide-refusal-{}-{kind}.pcraft", std::process::id()));
        std::fs::write(&path, b"Preserve destination")?;
        let error = session.execute("layer.exportAs", json!({"layer":id.0,"path":path.to_string_lossy()})).unwrap_err();
        assert!(error.to_string().contains("wide layer export"), "{error}");
        assert_eq!(std::fs::read(&path)?, b"Preserve destination");
        assert!(session.active().unwrap().doc.shares_state(&before));
        assert_eq!(session.active().unwrap().revision, revision);
        assert!(session.active().unwrap().history.can_redo());
    }
    Ok(())
}

#[test]
fn preparation_limits_cancellation_and_empty_refuse() -> Result<(), Box<dyn std::error::Error>> {
    use photocraft_engine::layer_export_wide::{Limits, extract};
    use photocraft_raster::Interrupt;
    let (mut doc, id) = control();
    assert!(extract(&doc, id, Limits::default(), &Interrupt::NONE).unwrap_err().to_string().contains("empty"));
    let raw: Vec<u8> = [1., 0., 0., 1.].into_iter().flat_map(f64::to_ne_bytes).collect();
    doc.layer_mut(id).unwrap().surface_mut().unwrap().try_write_interleaved(Rect::new(1, 1, 2, 2), &raw)?;
    let before = doc.clone();
    for limits in
        [Limits { peak_bytes: 1, work_units: u64::MAX }, Limits { peak_bytes: 15 << 20, work_units: u64::MAX }, Limits { peak_bytes: u64::MAX, work_units: 0 }]
    {
        assert!(extract(&doc, id, limits, &Interrupt::NONE).is_err());
    }
    assert!(extract(&doc, id, Limits::default(), &Interrupt::cancel_only(&|| true)).unwrap_err().to_string().contains("cancelled"));
    assert_eq!(doc, before);
    Ok(())
}

#[test]
fn wide_export_scaling_keeps_existing_checked_resampling() -> Result<(), Box<dyn std::error::Error>> {
    let (mut doc, id) = control();
    let raw: Vec<u8> = [1.25 + 2f64.powi(-38), 0.25, 0.5, 1.].into_iter().flat_map(f64::to_ne_bytes).collect();
    doc.layer_mut(id).unwrap().surface_mut().unwrap().try_write_interleaved(Rect::new(1, 1, 2, 2), &raw)?;
    let mut session = Session::new();
    session.add_document(doc, None)?;
    let before = session.active().unwrap().doc.clone();
    let path = std::env::temp_dir().join(format!("photocraft-wide-scale-{}.pcraft", std::process::id()));
    std::fs::write(&path, b"Preserve destination")?;
    let result = session.execute("layer.exportAs", json!({"layer":id.0,"path":path.to_string_lossy(),"scale":200}));
    result?;
    let saved = std::fs::read(path)?;
    let mut reopened = Session::new();
    open_bytes_as(&mut reopened, "scaled.pcraft", &saved, None, None)?;
    let current = &reopened.active().unwrap().doc;
    assert_eq!(current.size, Size::new(2, 2));
    let values = current.layers[0].surface().unwrap().try_read_interleaved(current.bounds())?;
    let first = f64::from_ne_bytes(values.as_chunks::<8>().0[0]);
    assert!((first - (1.25 + 2f64.powi(-38))).abs() < 1e-14, "{first:?}");
    assert!(session.active().unwrap().doc.shares_state(&before));
    Ok(())
}
