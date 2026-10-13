use photocraft_doc::{ColorMode, Document, ImportCompositing, Layer, SampleEncoding, SampleType, Size};
#[test]
fn imported_policy_falls_back_for_entire_document_even_when_hidden() {
    let mut d = Document::new("import", Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
    let mut l = Layer::raster("hidden imported", d.pixel_format());
    l.visible = false;
    l.import_compositing = Some(ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false });
    d.layers.push(Layer::group("nested", vec![l]));
    let e = photocraft_gpu::plan(&d).unwrap_err();
    assert!(e.0.contains("CPU compositor"));
    d.layers.clear();
    assert!(photocraft_gpu::plan(&d).is_ok());
}

#[test]
fn hardware_compositor_returns_fallback_before_imported_render() {
    let instance = wgpu::Instance::default();
    let adapter = match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("GPU unavailable for hardware fallback test: {e}");
            return;
        }
    };
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).unwrap();
    let format = photocraft_gpu::Compositor::preferred_acc_format(&adapter);
    let mut compositor = photocraft_gpu::Compositor::try_new_with_format(&device, format).unwrap();
    let mut d = Document::new("import", Size::new(1, 1), ColorMode::Rgb, SampleType::F32);
    d.sample_encoding = Some(SampleEncoding::Linear);
    let mut layer = Layer::raster("imported", d.pixel_format());
    layer.surface_mut().unwrap().write_pixel(0, 0, &[0.25, 0.5, 0.75, 1.0]);
    layer.import_compositing =
        Some(ImportCompositing { blend_encoding: SampleEncoding::Linear, composite_encoding: SampleEncoding::Linear, pass_through: false });
    d.layers.push(layer);
    assert!(compositor.supports(&d).unwrap_err().0.contains("CPU compositor"));
    assert!(photocraft_gpu::render_to_vec(&mut compositor, &device, &queue, &d, d.bounds()).unwrap_err().0.contains("CPU compositor"));
    assert_eq!(photocraft_compose::render(&d, d.bounds()).unwrap().px[0], [0.25, 0.5, 0.75, 1.0]);
}
