use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, LabelColor, Layer, Size};

#[test]
fn layer_labels_keep_the_psd_indices() {
    use photocraft_io::blocks::{label_from_index, label_index};
    let labels = [
        LabelColor::None,
        LabelColor::Red,
        LabelColor::Orange,
        LabelColor::Yellow,
        LabelColor::Green,
        LabelColor::Blue,
        LabelColor::Violet,
        LabelColor::Gray,
        LabelColor::Seafoam,
        LabelColor::Indigo,
        LabelColor::Magenta,
        LabelColor::Fuchsia,
    ];
    for (i, color) in labels.into_iter().enumerate() {
        assert_eq!(label_index(color), i as u16);
        assert_eq!(label_from_index(i as u16), color);
    }
    for i in [12, 255, u16::MAX] {
        assert_eq!(label_from_index(i), LabelColor::None);
    }
}

#[test]
fn layer_labels_psd_roundtrip_all_colors_and_depths() {
    for depth in SampleType::ALL {
        let mut doc = Document::new("Labels", Size::new(2, 2), ColorMode::Rgb, depth);
        for color in LabelColor::ALL.into_iter().filter(|color| *color != LabelColor::Brown) {
            let mut child = Layer::raster("Child", doc.pixel_format());
            child.label = color;
            let mut group = Layer::group(color.label(), vec![child]);
            group.label = color;
            doc.layers.push(group);
        }
        let bytes = photocraft_io::export(&doc, "labels.psd", &Default::default()).unwrap().bytes;
        let back = photocraft_io::import("labels.psd", &bytes).unwrap().document;
        let labels = |d: &Document| d.walk().into_iter().map(|(_, _, l)| (l.name.clone(), l.label)).collect::<Vec<_>>();
        assert_eq!(labels(&back), labels(&doc), "{depth:?}");
        let expected_depth = if matches!(depth, SampleType::U32 | SampleType::F64) { SampleType::F32 } else { depth };
        assert_eq!(back.depth, expected_depth);
    }
}

#[test]
fn layer_labels_truncated_sheet_color_is_not_a_label() {
    let block = photocraft_psd::TaggedBlock::new(*b"lclr", vec![0]);
    assert!(matches!(block.parsed(), Some(Err(_))));
}

#[test]
fn brown_psd_export_reports_loss_without_inventing_an_index_or_mutating_native_labels() {
    assert_eq!(photocraft_io::blocks::label_index_if_supported(LabelColor::Brown), None);
    for depth in [SampleType::U8,SampleType::U16,SampleType::F64] {
        let mut doc=Document::new("Brown",Size::new(2,2),ColorMode::Rgb,depth);
        let mut child=Layer::raster("Brown child",doc.pixel_format());child.label=LabelColor::Brown;
        let mut group=Layer::group("Brown group",vec![child]);group.label=LabelColor::Brown;doc.layers.push(group);
        let before=doc.clone();let exported=photocraft_io::export(&doc,"brown.psd",&Default::default()).unwrap();
        assert_eq!(exported.warnings.iter().filter(|w|w.contains("Brown layer label omitted from PSD")).count(),2);
        let reopened=photocraft_io::import("brown.psd",&exported.bytes).unwrap().document;
        assert!(reopened.walk().iter().all(|(_,_,layer)|layer.label==LabelColor::None));
        assert_eq!(doc,before);
    }
}
