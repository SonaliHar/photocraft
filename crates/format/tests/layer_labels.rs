use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, LabelColor, Layer, Size};

#[test]
fn layer_labels_pcraft_roundtrip_all_colors_and_depths() {
    for depth in SampleType::ALL {
        let mut doc = Document::new("Labels", Size::new(2, 2), ColorMode::Rgb, depth);
        for color in LabelColor::ALL {
            let mut child = Layer::raster("Child", doc.pixel_format());
            child.label = color;
            let mut group = Layer::group(color.label(), vec![child]);
            group.label = color;
            doc.layers.push(group);
        }
        let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
        assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(), doc, "{depth:?}");
        assert!(photocraft_format::load_from_bytes(&bytes[..bytes.len() / 2]).is_err());
    }
}

#[test]
fn brown_requires_reader_eight_only_for_documents_that_contain_it() {
    fn manifest(bytes:&[u8])->serde_json::Value {
        let zip=photocraft_format::zip::ZipReader::new(bytes).unwrap();
        let entry=zip.entries.iter().find(|e|e.name=="manifest.json").unwrap();
        serde_json::from_slice(&zip.read(entry,1<<20).unwrap()).unwrap()
    }
    let mut doc=Document::new("native floor",Size::new(2,2),ColorMode::Rgb,SampleType::U8);
    doc.layers.push(Layer::raster("ordinary",doc.pixel_format()));
    let old=photocraft_format::save_to_bytes(&doc,&Default::default()).unwrap();let old_manifest=manifest(&old);
    assert_eq!(old_manifest["minimum_reader_version"],2);
    assert_eq!(photocraft_format::load_from_bytes(&old).unwrap(),doc);
    doc.layers[0].label=LabelColor::Brown;
    let bytes=photocraft_format::save_to_bytes(&doc,&Default::default()).unwrap();let current=manifest(&bytes);
    assert_eq!(current["format_version"],8);assert_eq!(current["minimum_reader_version"],8);
    assert_eq!(photocraft_format::load_from_bytes(&bytes).unwrap(),doc);
    let zip=photocraft_format::zip::ZipReader::new(&bytes).unwrap();let mut writer=photocraft_format::zip::ZipWriter::new();
    for entry in &zip.entries {
        let mut data=zip.read(entry,1<<20).unwrap();
        if entry.name=="manifest.json" {let mut value:serde_json::Value=serde_json::from_slice(&data).unwrap();value["format_version"]=7.into();value["minimum_reader_version"]=7.into();data=serde_json::to_vec(&value).unwrap();}
        writer.add(&entry.name,&data).unwrap();
    }
    assert!(photocraft_format::load_from_bytes(&writer.finish().unwrap()).is_err());
}
