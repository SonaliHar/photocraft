use super::*;
use photocraft_doc::{CustomUnit, PhysicalResolution, PhysicalUnit};

use egui_kittest::kittest::Queryable;

type TestResult<T> = Result<T, Box<dyn std::error::Error>>;
fn custom() -> PhysicalUnit {
    PhysicalUnit::Custom(CustomUnit { name: "Info unit".into(), factor: 7.25, digits: 4, symbol: "iu".into(), abbreviation: "info".into() })
}
fn source(unit: &str, physical: Option<PhysicalResolution>, legacy: f32) -> TestResult<PhotocraftApp> {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width":288,"height":576,"background":"transparent"}))?;
    session.execute("select.rect", json!({"x":0,"y":0,"width":144,"height":288}))?;
    session.execute("prefs.set", json!({"path":"unitsAndRulers.rulers","value":unit}))?;
    session.edit("Info physical fixture", |doc, _| {
        doc.physical_resolution = physical;
        doc.resolution_dpi = legacy;
        Ok(())
    })?;
    let mut app = PhotocraftApp::new(session, Default::default());
    app.hover_doc = Some([144.0, 288.0]);
    Ok(app)
}
fn paint(app: &mut PhotocraftApp) -> Vec<String> {
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    let mut output = ctx
        .run_ui(egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(380.0, 260.0))), ..Default::default() }, |ui| {
            info_panel(app, ui)
        });
    output.textures_delta.clear();
    output.shapes.iter().filter_map(|shape| if let egui::Shape::Text(text) = &shape.shape { Some(text.galley.text().to_owned()) } else { None }).collect()
}
fn readouts(painted: &[String]) -> TestResult<Vec<String>> {
    ["X:", "Y:", "W:", "H:"]
        .into_iter()
        .map(|label| {
            let at = painted.iter().rposition(|s| s == label).ok_or("missing painted Info label")?;
            Ok(painted.get(at + 1).ok_or("missing painted Info value")?.clone())
        })
        .collect()
}
#[test]
fn info_physical_positions_and_selection_use_each_authoritative_axis() {
    for unit in [PhysicalUnit::Inch, custom()] {
        let mut app = source("inches", Some(PhysicalResolution { x_dpi: 144.0, y_dpi: 288.0, unit }), 72.0).unwrap();
        let before = app.session.active().unwrap().doc.clone();
        let rev = app.session.active().unwrap().revision;
        assert_eq!(readouts(&paint(&mut app)).unwrap(), vec!["1.000"; 4]);
        assert_eq!(app.session.active().unwrap().doc, before);
        assert_eq!(app.session.active().unwrap().revision, rev);
    }
}
#[test]
fn info_resolution_accessibility_uses_authoritative_axes() {
    let app = source("inches", Some(PhysicalResolution { x_dpi: 144.0, y_dpi: 288.0, unit: custom() }), 72.0).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(380.0, 260.0)).build_ui_state(|ui, app| info_panel(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(2);
    assert_eq!(h.get_all_by_label("1.000").count(), 4, "accessible Info values use canonical axes");
}
#[test]
fn info_resolution_invalid_metadata_has_no_bogus_physical_readouts() {
    for physical in [
        PhysicalResolution { x_dpi: 0.0, y_dpi: 288.0, unit: PhysicalUnit::Inch },
        PhysicalResolution { x_dpi: 144.0, y_dpi: f64::INFINITY, unit: PhysicalUnit::Inch },
        PhysicalResolution {
            x_dpi: 144.0,
            y_dpi: 288.0,
            unit: PhysicalUnit::Custom(CustomUnit { name: "invalid".into(), factor: 0.0, digits: 4, symbol: "i".into(), abbreviation: "i".into() }),
        },
    ] {
        let mut app = source("inches", Some(physical.clone()), 72.0).unwrap();
        assert_eq!(readouts(&paint(&mut app)).unwrap(), vec!["—"; 4]);
        for (unit, expected) in [("pixels", vec!["144", "288", "144", "288"]), ("percent", vec!["50.0"; 4])] {
            let mut app = source(unit, Some(physical.clone()), 72.0).unwrap();
            assert_eq!(readouts(&paint(&mut app)).unwrap(), expected);
        }
    }
}
#[test]
fn info_resolution_legacy_default_and_all_pref_units_preserve_formats() {
    for (unit, expected) in [
        ("pixels", vec!["144", "288", "144", "288"]),
        ("percent", vec!["50.0"; 4]),
        ("cm", vec!["2.54"; 4]),
        ("mm", vec!["25.40"; 4]),
        ("points", vec!["72.0"; 4]),
        ("picas", vec!["6.00"; 4]),
    ] {
        let mut app = source(unit, Some(PhysicalResolution { x_dpi: 144.0, y_dpi: 288.0, unit: custom() }), 72.0).unwrap();
        assert_eq!(readouts(&paint(&mut app)).unwrap(), expected);
    }
    for legacy in [72.0, 144.0] {
        let mut app = source("inches", None, legacy).unwrap();
        assert_eq!(
            readouts(&paint(&mut app)).unwrap(),
            vec![
                format!("{:.3}", 144.0 / f64::from(legacy)),
                format!("{:.3}", 288.0 / f64::from(legacy)),
                format!("{:.3}", 144.0 / f64::from(legacy)),
                format!("{:.3}", 288.0 / f64::from(legacy))
            ]
        );
    }
}
#[test]
#[ignore = "offscreen Vulkan visual evidence"]
fn info_resolution_offscreen_visual() {
    let app = source("inches", Some(PhysicalResolution { x_dpi: 144.0, y_dpi: 288.0, unit: custom() }), 72.0).unwrap();
    let mut setup = egui_kittest::wgpu::default_wgpu_setup();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut setup {
        create.instance_descriptor.backends = eframe::wgpu::Backends::VULKAN;
    }
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(380.0, 260.0))
        .with_pixels_per_point(2.0)
        .wgpu_setup(setup)
        .build_ui_state(|ui, app| info_panel(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(3);
    assert_eq!(h.get_all_by_label("1.000").count(), 4);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../temp/sol3-info-physical-resolution-20261012/info-canonical.png");
    h.render().unwrap().save(path).unwrap();
}

#[test]
fn info_resolution_tiny_valid_axes_are_not_silently_clamped() {
    let mut app = source("inches", Some(PhysicalResolution { x_dpi: 1e-7, y_dpi: 2e-7, unit: custom() }), 72.0).unwrap();
    assert_eq!(readouts(&paint(&mut app)).unwrap(), vec!["1440000000.000"; 4]);
    let mut app = source("inches", Some(PhysicalResolution { x_dpi: f64::MIN_POSITIVE, y_dpi: f64::MIN_POSITIVE, unit: custom() }), 72.0).unwrap();
    assert_eq!(readouts(&paint(&mut app)).unwrap(), vec!["—"; 4]);
    let mut app = source("inches", None, 0.0).unwrap();
    assert_eq!(readouts(&paint(&mut app)).unwrap(), vec!["—"; 4]);
}
