use super::*;
use photocraft_doc::{Knot, NamedPath, Path, Subpath};

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-print-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

fn session(mode: &str, depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 150, "mode": mode, "depth": depth, "name": "Print Me.psd", "background": "#3366cc"})).unwrap();
    s
}

fn find(h: &[u8], n: &[u8], from: usize) -> usize {
    from + h[from..].windows(n.len()).position(|w| w == n).unwrap()
}

/// The (inflated) image stream of a print PDF: (width, height, colour space, samples).
fn pdf_image(pdf: &[u8]) -> (u32, u32, String, Vec<u8>) {
    let i = find(pdf, b"/Subtype /Image", 0);
    let end = find(pdf, b">>\nstream\n", i);
    let dict = String::from_utf8_lossy(&pdf[i..end]).into_owned();
    let num = |k: &str| dict.split(k).nth(1).unwrap().split_whitespace().next().unwrap().parse::<u32>().unwrap();
    let cs = dict.split("/ColorSpace ").nth(1).unwrap().split(" /BitsPerComponent").next().unwrap().to_string();
    let len = num("/Length ") as usize;
    let start = end + 10;
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&pdf[start..start + len]), &mut out).unwrap();
    (num("/Width "), num("/Height "), cs, out)
}

#[test]
fn print_dry_run_renders_a_pdf_and_reports_the_platform_backend() {
    let dir = tmp("print");
    for depth in [8, 16, 32] {
        let mut s = session("rgb", depth);
        let out = format!("{dir}/rgb-{depth}.pdf");
        let r = s
            .execute(
                "file.print",
                json!({"output": out, "send": true, "dryRun": true, "printer": "Office", "copies": 2, "paper": "a4", "cornerCropMarks": true, "registrationMarks": true, "labels": true}),
            )
            .unwrap();
        let cmd: Vec<String> = serde_json::from_value(r["command"].clone()).unwrap();
        #[cfg(unix)]
        {
            assert_eq!(&cmd[..5], &["lp", "-d", "Office", "-n", "2"]);
            assert!(cmd.contains(&"fit-to-page=false".into()));
            assert!(!cmd.contains(&out), "PDF bytes are streamed instead of passing a path");
        }
        #[cfg(windows)]
        assert_eq!(cmd.first().map(String::as_str), Some("xps-print"));
        assert_eq!(r["sent"], false);
        assert_eq!(r["pdf"], out);
        let pdf = std::fs::read(&out).unwrap();
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(String::from_utf8_lossy(&pdf).contains("/MediaBox [0 0 595.28 841.89]"));
        let (w, h, cs, data) = pdf_image(&pdf);
        assert_eq!((w, h), (300, 150));
        assert_eq!(cs, "/DeviceRGB");
        assert_eq!(&data[..3], &[0x33, 0x66, 0xcc]);
        // Auto-rotation fits the landscape source to the portrait A4 page: 150 × 300 pt.
        let rect: Vec<f64> = serde_json::from_value(r["imageRect"].clone()).unwrap();
        assert!((rect[2] - 150.0).abs() < 1e-6 && (rect[3] - 300.0).abs() < 1e-6);
        assert!((rect[0] - (595.28 - 150.0) / 2.0).abs() < 1e-6);
    }
    // Print to PDF: no spooling by default.
    let mut s = session("gray", 8);
    let out = format!("{dir}/out.pdf");
    let r = s.execute("file.print", json!({"output": out, "scaleToFit": true, "orientation": "landscape"})).unwrap();
    assert_eq!(r["sent"], false);
    assert!(r["command"].is_null());
    let (_, _, cs, _) = pdf_image(&std::fs::read(&out).unwrap());
    assert_eq!(cs, "/DeviceGray");
    let scale = r["scale"].as_f64().unwrap();
    assert!(scale > 200.0, "fit to a landscape letter page: {scale}");
    assert!(s.execute("file.print", json!({"paper": "napkin", "dryRun": true})).is_err());
    assert!(s.execute("file.print", json!({"colorHandling": "photocraftManages", "dryRun": true})).is_err(), "needs a printer profile");

    for depth in [8, 16, 32] {
        let mut s = session("rgb", depth);
        let out = format!("{dir}/forced-gray-{depth}.pdf");
        let r = s.execute("file.print", json!({"output": out, "grayscale": true})).unwrap();
        let (_, _, cs, data) = pdf_image(&std::fs::read(r["pdf"].as_str().unwrap()).unwrap());
        assert_eq!(cs, "/DeviceGray");
        assert_eq!(data.len(), 300 * 150);
        assert!((i16::from(data[0]) - 99).abs() <= 1, "#3366cc luminance at {depth}-bit: {}", data[0]);
        let preview = preview_thumbnail(&s.active().unwrap().doc, true, 400);
        assert_eq!(preview.pixels[0], preview.pixels[1]);
        assert_eq!(preview.pixels[1], preview.pixels[2]);
        assert_eq!(preview.pixels[3], 255);
        let colour = preview_thumbnail(&s.active().unwrap().doc, false, 400);
        assert_eq!(&colour.pixels[..4], &[0x33, 0x66, 0xcc, 255]);
    }
}

#[test]
fn photocraft_manages_colors_converts_to_the_printer_profile() {
    let mut s = session("rgb", 8);
    let out = format!("{}/managed.pdf", tmp("managed"));
    let r = s
        .execute(
            "file.print",
            json!({"output": out, "dryRun": true, "colorHandling": "photocraftManages", "printerProfile": "coated-cmyk", "intent": "perceptual", "bpc": false}),
        )
        .unwrap();
    assert_eq!(r["color"]["intent"], "perceptual");
    let pdf = std::fs::read(&out).unwrap();
    let (_, _, cs, data) = pdf_image(&pdf);
    assert_eq!(cs, "[/ICCBased 7 0 R]");
    assert!(String::from_utf8_lossy(&pdf).contains("/N 4 /Alternate /DeviceCMYK"));
    assert_eq!(data.len(), 300 * 150 * 4);
    // Blue: lots of cyan, little yellow.
    assert!(data[0] > 150 && data[2] < 60, "{:?}", &data[..4]);
    // The document itself is untouched.
    assert_eq!(s.active().unwrap().doc.mode, ColorMode::Rgb);
}

/// The inflated page content stream (object 4).
fn pdf_content(pdf: &[u8]) -> String {
    let i = find(pdf, b"4 0 obj\n", 0);
    let len_at = find(pdf, b"/Length ", i) + 8;
    let len: usize = String::from_utf8_lossy(&pdf[len_at..len_at + 12]).split_whitespace().next().unwrap().parse().unwrap();
    let start = find(pdf, b"stream\n", i) + 7;
    let mut out = String::new();
    std::io::Read::read_to_string(&mut flate2::read::ZlibDecoder::new(&pdf[start..start + len]), &mut out).unwrap();
    out
}

#[test]
fn emulsion_down_mirrors_the_whole_page() {
    let mut s = session("rgb", 8);
    let dir = tmp("emulsion");
    let plain_path = format!("{dir}/plain.pdf");
    let mirrored_path = format!("{dir}/mirrored.pdf");
    let again_path = format!("{dir}/again.pdf");
    let plain = s
        .execute("file.print", json!({"output": plain_path, "dryRun": true, "paper": "a4", "center": false, "left": 1.0, "top": 1.0, "labels": true}))
        .unwrap();
    let mirrored = s
        .execute(
            "file.print",
            json!({"output": mirrored_path, "dryRun": true, "paper": "a4", "center": false, "left": 1.0, "top": 1.0, "labels": true, "emulsionDown": true}),
        )
        .unwrap();
    assert_eq!(plain["emulsionDown"], false);
    assert_eq!(mirrored["emulsionDown"], true);
    // Same layout; only the page is flipped.
    assert_eq!(plain["imageRect"], mirrored["imageRect"]);
    let (p, m) = (std::fs::read(plain["pdf"].as_str().unwrap()).unwrap(), std::fs::read(mirrored["pdf"].as_str().unwrap()).unwrap());
    let (pc, mc) = (pdf_content(&p), pdf_content(&m));
    assert!(!pc.contains("-1 0 0 1"));
    // The flip wraps the image and the label, after the white paper, and is closed again.
    let flip = mc.find("q -1 0 0 1 595.28 0 cm").expect("mirror transform");
    assert!(flip > mc.find("re f Q").unwrap() && flip < mc.find("/Im0 Do").unwrap() && flip < mc.find("Tj").unwrap());
    assert_eq!(mc.matches("q ").count() + mc.matches("q\n").count(), mc.matches("Q\n").count(), "balanced q/Q: {mc}");
    // The pixels themselves are not touched (the transform mirrors them).
    assert_eq!(pdf_image(&p).3, pdf_image(&m).3);
    // Print One Copy keeps it.
    let again = s.execute("file.printOneCopy", json!({"output": again_path, "dryRun": true})).unwrap();
    assert_eq!(again["emulsionDown"], true);
    for r in [plain, mirrored, again] {
        let _ = std::fs::remove_file(r["pdf"].as_str().unwrap());
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn page_of(channels: usize, data: Vec<u8>, emulsion_down: bool) -> PrintPage {
    PrintPage {
        paper: (600.0, 800.0),
        image: PrintImage { width: 2, height: 1, channels, data, icc: None },
        rect: (100.0, 200.0, 144.0, 72.0),
        marks: Marks::default(),
        rotate_image: false,
        description: None,
        label: None,
        emulsion_down,
    }
}

#[test]
fn windows_print_image_is_rgb_mirrored_for_emulsion_down() {
    assert_eq!(page_rgb(&page_of(3, vec![1, 2, 3, 4, 5, 6], false)).unwrap(), [1, 2, 3, 4, 5, 6]);
    assert_eq!(page_rgb(&page_of(3, vec![1, 2, 3, 4, 5, 6], true)).unwrap(), [4, 5, 6, 1, 2, 3]);
    assert_eq!(page_rgb(&page_of(1, vec![7, 9], true)).unwrap(), [9, 9, 9, 7, 7, 7]);
    assert!(page_rgb(&page_of(4, vec![0; 8], false)).is_err(), "CMYK can't go to a GDI printer");
    assert!(page_rgb(&page_of(3, vec![0; 5], false)).is_err(), "short data");
    // GDI's origin is the sheet's top-left; Emulsion Down moves the box to the mirrored side.
    assert_eq!(page_rect_top_left(&page_of(3, vec![0; 6], false)), (100.0, 528.0, 144.0, 72.0));
    assert_eq!(page_rect_top_left(&page_of(3, vec![0; 6], true)), (356.0, 528.0, 144.0, 72.0));
}

#[test]
fn native_rgb_route_preserves_pdf_only_color_and_page_content() {
    let mut page = page_of(3, vec![0; 6], false);
    assert!(native_rgb_compatible(&page, "printerManages"));
    page.label = Some("Print Me".into());
    assert!(!native_rgb_compatible(&page, "printerManages"));
    page.label = None;
    page.description = Some("Proof description".into());
    assert!(!native_rgb_compatible(&page, "printerManages"));
    page.description = Some(String::new());
    assert!(native_rgb_compatible(&page, "printerManages"));
    page.image.icc = Some(vec![1, 2, 3]);
    assert!(!native_rgb_compatible(&page, "printerManages"), "preserve embedded source ICC in the PDF route");
    assert!(native_rgb_compatible(&page, "photocraftManages"));
    assert!(native_rgb_compatible(&page, "photoshopManages"));
    assert!(!native_rgb_compatible(&page_of(4, vec![0; 8], false), "photocraftManages"));
    assert!(native_rgb_compatible(&page_of(1, vec![0; 2], false), "noColorManagement"));
}

#[test]
fn printer_commands_answer_without_a_document_and_reject_bad_settings() {
    let mut s = Session::new();
    let r = s.execute("file.print.printers", json!({})).unwrap();
    assert_eq!(r["supported"], cfg!(windows));
    assert!(r["printers"].is_array());
    assert!(s.execute("file.print.page", json!({"printer": "Anything", "printerSettings": "zz"})).is_err());
    let mut s = session("rgb", 8);
    assert!(s.execute("file.print", json!({"printer": "Anything", "printerSettings": "0102", "send": true})).is_err());
}

#[test]
fn printer_profile_can_be_an_installed_icc_file() {
    let dir = tmp("icc");
    let icc = format!("{dir}/My Paper.icc");
    let output = format!("{dir}/managed.pdf");
    std::fs::write(&icc, photocraft_cms::Builtin::CoatedCmyk.profile().to_bytes().as_slice()).unwrap();
    let mut s = session("rgb", 16);
    let r = s
        .execute("file.print", json!({"output": output, "dryRun": true, "colorHandling": "photocraftManages", "printerProfile": icc, "emulsionDown": true}))
        .unwrap();
    assert_eq!(r["color"]["printerProfile"], json!(icc));
    let (_, _, cs, data) = pdf_image(&std::fs::read(r["pdf"].as_str().unwrap()).unwrap());
    assert_eq!(cs, "[/ICCBased 7 0 R]");
    assert_eq!(data.len(), 300 * 150 * 4);
    assert!(s.execute("file.print", json!({"dryRun": true, "colorHandling": "photocraftManages", "printerProfile": format!("{dir}/missing.icc")})).is_err());
    let _ = std::fs::remove_file(r["pdf"].as_str().unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn print_one_copy_repeats_the_last_settings() {
    let mut s = session("rgb", 8);
    let first = s.execute("file.print", json!({"dryRun": true, "copies": 3, "paper": "a5", "scale": 50})).unwrap();
    assert!(first["pdf"].is_null());
    let r = s.execute("file.printOneCopy", json!({"dryRun": true})).unwrap();
    assert_eq!(r["copies"], 1);
    assert_eq!(r["paper"], json!([419.53, 595.28]));
    assert_eq!(r["scale"], 50.0);
    let cmd: Vec<String> = serde_json::from_value(r["command"].clone()).unwrap();
    #[cfg(unix)]
    assert_eq!(&cmd[..3], &["lp", "-n", "1"]);
    #[cfg(windows)]
    assert_eq!(cmd.first().map(String::as_str), Some("xps-print"));
    assert!(r["pdf"].is_null());
}

#[test]
fn system_dialog_receives_pdf_and_reports_confirmation_or_cancel() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let calls = Arc::new(AtomicUsize::new(0));
    let seen = Arc::clone(&calls);
    let mut s = session("rgb", 8);
    s.system_print_dialog = Some(Arc::new(move |pdf, title| {
        assert!(pdf.starts_with(b"%PDF"));
        assert_eq!(title, "Print Me.psd");
        seen.fetch_add(1, Ordering::Relaxed);
        Ok(SystemPrintOutcome::Printed)
    }));
    let printed = s.execute("file.print", json!({"systemDialog": true})).unwrap();
    assert_eq!(printed["sent"], true);
    assert_eq!(printed["spooler"], "system-dialog");
    assert_eq!(calls.load(Ordering::Relaxed), 1);

    s.system_print_dialog = Some(Arc::new(|_, _| Ok(SystemPrintOutcome::Cancelled)));
    let cancelled = s.execute("file.print", json!({"systemDialog": true})).unwrap();
    assert_eq!(cancelled["cancelled"], true);
    assert_eq!(cancelled["sent"], false);
}

#[test]
fn system_dialog_requires_desktop_bridge_and_print_one_copy_bypasses_it() {
    use std::sync::Arc;

    let mut s = session("rgb", 8);
    assert!(s.execute("file.print", json!({"systemDialog": true})).is_err());
    s.system_print_dialog = Some(Arc::new(|_, _| Err("dialog must not open".into())));
    let first = s.execute("file.print", json!({"dryRun": true, "systemDialog": true, "paper": "a5"})).unwrap();
    assert_eq!(first["systemDialog"], true);
    let repeated = s.execute("file.printOneCopy", json!({"dryRun": true})).unwrap();
    assert_eq!(repeated["systemDialog"], false);
}

#[test]
fn package_copies_links_and_relinks() {
    let dir = tmp("package");
    let link = format!("{dir}/art.png");
    let mut src = Session::new();
    src.execute("file.new", json!({"width": 20, "height": 20, "background": "#ff0000"})).unwrap();
    crate::file_cmds::save_doc(&src.active().unwrap().doc, &link, None).unwrap();
    let mut s = session("rgb", 8);
    s.execute("file.placeLinked", json!({"path": link})).unwrap();
    let r = s.execute("file.package", json!({"dir": format!("{dir}/pkg")})).unwrap();
    let copied = format!("{dir}/pkg/Print Me/Links/art.png");
    assert_eq!(r["links"], json!([copied]));
    assert!(std::path::Path::new(&copied).is_file());
    let docp = r["document"].as_str().unwrap();
    assert!(docp.ends_with("Print Me/Print Me.pcraft"));
    let back = photocraft_io::import("Print Me.pcraft", &std::fs::read(docp).unwrap()).unwrap().document;
    let linked: Vec<String> = back
        .walk()
        .iter()
        .filter_map(|(_, _, l)| match &l.content {
            LayerContent::Smart(so) => match &so.source {
                SmartSource::Linked { path } => Some(path.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect();
    assert_eq!(linked.len(), 1);
    assert!(linked[0].ends_with("Links/art.png"), "{linked:?}");
    // The open document still points at the original.
    let open = &s.active().unwrap().doc;
    assert!(
        open.walk()
            .iter()
            .any(|(_, _, l)| matches!(&l.content, LayerContent::Smart(so) if matches!(&so.source, SmartSource::Linked { path } if *path == link)))
    );
}

#[test]
fn paths_export_as_illustrator_postscript() {
    let mut s = session("rgb", 8);
    assert!(!s.is_enabled("file.export.pathsToIllustrator"));
    s.edit("paths", |d, _| {
        let tri = Subpath::polygon(&[(10.0, 10.0), (100.0, 10.0), (50.0, 90.0)]);
        let mut curve = Subpath::polyline(&[(0.0, 0.0), (300.0, 150.0)]);
        curve.knots[0] = Knot::smooth(photocraft_geom::Point::new(0.0, 0.0), photocraft_geom::Point::new(0.0, 0.0), photocraft_geom::Point::new(50.0, 0.0));
        d.paths.push(NamedPath { name: "Shapes".into(), path: Path::new(vec![tri, curve]), psd_raw: None });
        Ok(())
    })
    .unwrap();
    let r = s.execute("file.export.pathsToIllustrator", json!({})).unwrap();
    let ai = r["ai"].as_str().unwrap();
    assert!(ai.starts_with("%!PS-Adobe-2.0 EPSF-1.2"));
    assert!(ai.contains("%%BoundingBox: 0 0 300 150"));
    assert!(ai.contains("*u\n10.0000 140.0000 m\n100.0000 140.0000 L\n"), "{ai}");
    assert!(ai.contains("\nn\n") && ai.contains("\nN\n*U\n"));
    assert!(ai.contains(" C\n"), "curve segment");
    let dir = tmp("ai");
    let r = s.execute("file.export.pathsToIllustrator", json!({"path": format!("{dir}/p.ai"), "paths": "Shapes"})).unwrap();
    assert_eq!(r["paths"], 1);
    assert!(s.execute("file.export.pathsToIllustrator", json!({"paths": "Nope"})).is_err());
}

#[test]
fn print_dry_run_without_output_retains_no_pdf() {
    let mut s = session("rgb", 8);
    let result = s.execute("file.print", json!({"dryRun": true})).unwrap();
    assert!(result["pdf"].is_null());
    assert_eq!(result["sent"], false);
    assert!(result["bytes"].as_u64().unwrap() > 100);
    assert!(result["command"].is_array());
    assert!(s.file_menu.last_print.as_ref().unwrap().get("dryRun").is_none());
}

#[test]
fn print_dry_run_preserves_system_printer_job_settings() {
    let mut s = session("rgb", 8);
    let result = s
        .execute(
            "file.print",
            json!({
                "dryRun": true,
                "printer": "Office Printer",
                "copies": 2,
                "collate": false,
                "duplex": "shortEdge",
                "grayscale": true,
                "driverOptions": {"InputSlot": "Tray2"}
            }),
        )
        .unwrap();
    assert_eq!(result["printer"], "Office Printer");
    assert_eq!(result["copies"], 2);
    assert_eq!(result["collate"], false);
    assert_eq!(result["duplex"], "shortEdge");
    assert_eq!(result["grayscale"], true);
    assert_eq!(result["driverOptions"], json!([["InputSlot", "Tray2"], ["media", "na_letter_8.5x11in"], ["orientation-requested", "3"]]));
    #[cfg(unix)]
    {
        let command: Vec<String> = serde_json::from_value(result["command"].clone()).unwrap();
        assert!(command.windows(2).any(|pair| pair == ["-d", "Office Printer"]));
        assert!(command.iter().any(|argument| argument == "collate=false"));
        assert!(command.iter().any(|argument| argument == "sides=two-sided-short-edge"));
        assert!(command.iter().any(|argument| argument == "print-color-mode=monochrome"));
        assert!(command.iter().any(|argument| argument == "InputSlot=Tray2"));
    }
}

#[test]
fn c5_envelope_selects_c5_media_and_keeps_requested_orientation() {
    let mut s = session("rgb", 8);
    let dir = tmp("c5-orientation");
    for (orientation, expected_paper) in [("portrait", [459.21, 649.13]), ("landscape", [649.13, 459.21])] {
        let output = format!("{dir}/{orientation}.pdf");
        let result = s.execute("file.print", json!({"paper": "c5", "orientation": orientation, "output": output, "send": true, "dryRun": true})).unwrap();
        let paper: Vec<f64> = serde_json::from_value(result["paper"].clone()).unwrap();
        assert!((paper[0] - expected_paper[0]).abs() < 0.01, "{orientation}: {paper:?}");
        assert!((paper[1] - expected_paper[1]).abs() < 0.01, "{orientation}: {paper:?}");
        let should_rotate = orientation == "portrait";
        assert_eq!(result["rotateImage"], should_rotate);
        let pdf = std::fs::read(result["pdf"].as_str().unwrap()).unwrap();
        let content = pdf_content(&pdf);
        if should_rotate {
            assert!(content.contains("q 0 300.000 -150.000 0"), "{content}");
        } else {
            assert!(content.contains("q 300.000 0 0 150.000"), "{content}");
        }
        #[cfg(unix)]
        {
            let command: Vec<String> = serde_json::from_value(result["command"].clone()).unwrap();
            assert!(command.iter().any(|argument| argument == "media=iso_c5_162x229mm"), "{command:?}");
            let requested_orientation = if orientation == "landscape" { "orientation-requested=4" } else { "orientation-requested=3" };
            assert!(command.iter().any(|argument| argument == requested_orientation), "{command:?}");
        }
    }
    let fixed_orientation = s.execute("file.print", json!({"paper": "c5", "orientation": "portrait", "autoRotate": false, "dryRun": true})).unwrap();
    assert_eq!(fixed_orientation["rotateImage"], false);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn print_without_sending_requires_output_unless_dry_run_and_forgets_send_override() {
    let mut s = session("rgb", 8);
    let before = s.file_menu.last_print.clone();
    let err = s.execute("file.print", json!({"send": false})).unwrap_err();
    assert!(err.to_string().contains("needs an output path"), "{err}");
    assert_eq!(s.file_menu.last_print, before);
    let result = s.execute("file.print", json!({"send": false, "dryRun": true})).unwrap();
    assert!(result["pdf"].is_null());
    assert!(result["command"].is_null());
    assert!(s.file_menu.last_print.as_ref().unwrap().get("send").is_none());
    let out = format!("{}/export.pdf", tmp("non-sending-export"));
    let result = s.execute("file.print", json!({"output": out, "send": false})).unwrap();
    assert_eq!(result["sent"], false);
    assert!(std::fs::read(&out).unwrap().starts_with(b"%PDF-1.4"));
    assert!(s.file_menu.last_print.as_ref().unwrap().get("send").is_none());
    s.execute("file.print", json!({"output": out, "send": true, "dryRun": true})).unwrap();
    assert!(s.file_menu.last_print.as_ref().unwrap().get("send").is_none());
}
/// parallel tests are untouched, and the real spool boundary still consumes the PDF bytes.
#[cfg(unix)]
#[test]
fn spool_transport_streams_pdf_and_propagates_refusal() {
    const CHILD: &str = "PHOTOCRAFT_TEST_PRINT_TRANSPORT";
    if let Ok(dir) = std::env::var(CHILD) {
        let mut s = session("rgb", 8);
        let source = s.active().unwrap().doc.id;
        s.execute("file.scripts.scriptEventsManager", json!({"add": {"event": "print", "steps": [["image.imageRotation.90cw", {}]]}})).unwrap();
        let crate::jobs::Started::Job(job) = s.start("file.print", json!({"printer": "Office", "copies": 3, "paper": "a4", "scale": 50})).unwrap() else {
            panic!("desktop printing must start on a worker");
        };
        assert!(!s.cancel_job(job), "queue side effects must not be reported as cancelled");
        assert_eq!(s.execute("jobs.cancel", json!({})).unwrap()["cancelled"], 0);
        assert!(s.execute("jobs.cancel", json!({"job": job.0})).unwrap_err().to_string().contains("system print queue"));
        s.execute("file.new", json!({"width": 40, "height": 10})).unwrap();
        let viewed = s.active().unwrap().doc.id;
        let journal_before_completion = s.journal.len();
        let out = s.wait_job(job).unwrap();
        assert_eq!(s.journal.len(), journal_before_completion + 1, "print hooks must not become duplicate recorded action steps");
        assert_eq!(s.journal.last().unwrap().0, "file.print");
        assert_eq!(s.active().unwrap().doc.id, viewed, "completion keeps the user's tab");
        assert_eq!(s.active().unwrap().doc.size.width, 40, "print hook must not act on the newly viewed document");
        let printed = &s.documents().iter().find(|st| st.doc.id == source).unwrap().doc;
        assert_eq!((printed.size.width, printed.size.height), (150, 300), "print hook ran on the source document");
        assert_eq!(out["sent"], true);
        assert_eq!(out["spooler"], "request id is Office-42");
        assert!(out["pdf"].is_null(), "spool-only printing leaves no diagnostic PDF");
        let pdf = std::fs::read(format!("{dir}/received.pdf")).unwrap();
        assert!(pdf.starts_with(b"%PDF-1.4"));
        assert!(String::from_utf8_lossy(&pdf).contains("/MediaBox [0 0 595.28 841.89]"));
        let args = std::fs::read_to_string(format!("{dir}/args")).unwrap();
        assert!(args.contains("fit-to-page=false"));
        assert!(args.contains("-n\n3\n"));
        assert!(!args.contains(".pdf"));
        let remembered = s.file_menu.last_print.clone();
        let explicit = format!("{dir}/saved.pdf");
        let err = s.execute("file.print", json!({"printer": "Refuse", "output": explicit, "send": true})).unwrap_err();
        assert!(err.to_string().contains("queue refused"), "{err}");
        assert!(std::fs::read(&explicit).unwrap().starts_with(b"%PDF-1.4"), "explicit export survives a spool refusal");
        assert_eq!(s.file_menu.last_print, remembered, "failed printing does not replace repeat settings");
        // Export-only choices must not disable submission when Print One Copy repeats settings.
        let export = format!("{dir}/export-only.pdf");
        s.execute("file.print", json!({"printer": "Office", "output": export, "send": false})).unwrap();
        let remembered = s.file_menu.last_print.as_ref().unwrap();
        assert!(remembered.get("send").is_none() && remembered.get("output").is_none());
        let repeat = s.execute("file.printOneCopy", json!({})).unwrap();
        assert_eq!(repeat["sent"], true);
        assert_eq!(repeat["copies"], 1);
        assert!(repeat["pdf"].is_null());
        assert!(!std::fs::read_to_string(format!("{dir}/args")).unwrap().contains(".pdf"));
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("lp");
    std::fs::write(
        &script,
        b"#!/bin/sh\nif [ \"$2\" = Refuse ]; then echo 'queue refused' >&2; exit 1; fi\nprintf '%s\\n' \"$@\" > \"$PHOTOCRAFT_TEST_PRINT_TRANSPORT/args\"\n/bin/cat > \"$PHOTOCRAFT_TEST_PRINT_TRANSPORT/received.pdf\"\necho 'request id is Office-42 (0 file(s))'\n",
    ).unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o700)).unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "print_cmds::tests::spool_transport_streams_pdf_and_propagates_refusal", "--nocapture"])
        .env(CHILD, dir.path())
        .env("PATH", dir.path())
        // The child spools into the fake `lp` above, so a shell that blocks real printing
        // (`PHOTOCRAFT_NO_PRINT`, see `disabled_printing_never_spools`) must not stop it.
        .env_remove("PHOTOCRAFT_NO_PRINT")
        .output()
        .unwrap();
    assert!(result.status.success(), "{}\n{}", String::from_utf8_lossy(&result.stdout), String::from_utf8_lossy(&result.stderr));
    assert!(String::from_utf8_lossy(&result.stdout).contains("1 passed"), "child must select the transport test");
}

#[test]
fn disabled_printing_never_spools() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    super::disable_printing();
    assert!(super::printing_disabled());
    let e = s.execute("file.print", json!({})).unwrap_err().to_string();
    assert!(e.contains("printing is disabled"), "{e}");
    assert!(s.execute("file.printOneCopy", json!({})).is_err());
    // A dry run still renders and reports the command it would have run.
    let r = s.execute("file.print", json!({"dryRun": true})).unwrap();
    assert_eq!(r["sent"], false);
    assert!(r["command"].is_array(), "{r}");
}

#[test]
fn printers_lists_the_spooler_destinations_without_a_document() {
    let mut s = Session::new();
    let r = s.execute("file.printers", json!({})).unwrap();
    let list = r["printers"].as_array().unwrap();
    assert!(list.iter().all(|p| p["name"].is_string() && p["default"].is_boolean()));
    match r["default"].as_str() {
        Some(d) => assert!(list.iter().any(|p| p["name"] == d && p["default"] == true)),
        None => assert!(list.iter().all(|p| p["default"] == false)),
    }
}

#[test]
fn rotated_print_resolution_tracks_scale_without_resampling() {
    let s = session("rgb", 16);
    let doc = &s.active().unwrap().doc;
    for rotate in [false, true] {
        for scale in [50.0, 100.0, 200.0] {
            let l = layout(doc, &json!({"autoRotate": rotate, "scale": scale}), "file.print").unwrap();
            assert_eq!(l.rotate_image, rotate);
            assert!((l.resolution_ppi(doc) - f64::from(doc.resolution_dpi) * 100.0 / scale).abs() < 1e-6);
        }
    }
}

#[test]
fn invalid_print_geometry_is_rejected_without_output_or_remembered_settings() {
    let mut s = session("rgb", 8);
    let dir = tmp("bad-geometry");
    let output = format!("{dir}/invalid.pdf");
    for mut params in [
        json!({"paper": [1e300, 792]}),
        json!({"paper": [40, 40], "cornerCropMarks": true, "scaleToFit": true}),
        json!({"scale": 0}),
        json!({"scale": -10}),
        json!({"scale": "wrong"}),
        json!({"scale": 1e300}),
        json!({"center": false, "top": 1e300}),
        json!({"center": false, "left": "wrong"}),
        json!({"orientation": "wrong"}),
    ] {
        params["output"] = json!(output);
        assert!(s.execute("file.print", params.clone()).is_err(), "{params}");
        assert!(!std::path::Path::new(&output).exists());
        assert!(s.file_menu.last_print.is_none());
    }
    let mut doc = (*s.active().unwrap().doc).clone();
    for dpi in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        doc.resolution_dpi = dpi;
        assert!(layout(&doc, &json!({}), "file.print").is_err());
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn paper_selection_overrides_conflicting_driver_media() {
    let mut s = session("rgb", 8);
    for (paper, media) in [(json!("a4"), "iso_a4_210x297mm"), (json!("5x7"), "na_5x7_5x7in"), (json!([576, 432]), "Custom.432.00x576.00pt")] {
        let r = s
            .execute(
                "file.print",
                json!({"paper": paper, "orientation": "landscape", "dryRun": true,
                    "driverOptions": {"media": "Letter", "PageSize": "Letter", "PageRegion": "Letter", "InputSlot": "Tray2"}
                }),
            )
            .unwrap();
        let options: Vec<(String, String)> = serde_json::from_value(r["driverOptions"].clone()).unwrap();
        assert_eq!(options.iter().filter(|(key, _)| key == "media").count(), 1);
        assert!(options.contains(&("media".into(), media.into())));
        assert!(options.contains(&("InputSlot".into(), "Tray2".into())));
        assert!(!options.iter().any(|(key, _)| key == "PageSize" || key == "PageRegion"));
    }
    let doc = &s.active().unwrap().doc;
    assert_eq!(layout(doc, &json!({"paper": [576, 432], "orientation": "portrait"}), "file.print").unwrap().paper, (432.0, 576.0));
}

#[test]
fn direct_print_paper_limits_match_each_backend_but_pdf_export_remains_portable() {
    use craft_print::Backend;
    for backend in [Some(Backend::Cups), Some(Backend::WindowsPdf)] {
        assert!(validate_spool_paper((612.0, 792.0), backend, "file.print").is_ok());
        assert!(validate_spool_paper((9216.0, 9216.0), backend, "file.print").is_ok());
        assert_eq!(validate_spool_paper((10000.0, 12000.0), backend, "file.print").is_ok(), backend == Some(Backend::Cups));
    }
    let mut s = session("rgb", 8);
    // PDF-only output is valid even where direct Windows printing has a smaller sheet limit.
    let r = s.execute("file.print", json!({"paper": [10000.0, 12000.0], "send": false, "dryRun": true})).unwrap();
    assert_eq!(r["paper"], json!([10000.0, 12000.0]));
    assert_eq!(r["sent"], false);
}

#[test]
fn native_backend_diagnostics_are_available_on_each_supported_desktop() {
    let mut s = session("rgb", 8);
    let result = s.execute("file.print", json!({"dryRun": true, "printer": "Office café & photo", "copies": 2})).unwrap();
    assert_eq!(result["sent"], false);
    assert_eq!(result["printer"], "Office café & photo");
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        assert_eq!(craft_print::backend(), Some(craft_print::Backend::Cups));
        assert!(!has_system_printer_preferences());
        let command: Vec<String> = serde_json::from_value(result["command"].clone()).unwrap();
        assert_eq!(&command[..5], &["lp", "-d", "Office café & photo", "-n", "2"]);
    }
    #[cfg(windows)]
    {
        assert_eq!(craft_print::backend(), Some(craft_print::Backend::WindowsPdf));
        assert!(has_system_printer_preferences());
        assert!(result["command"].as_array().unwrap().last().unwrap().as_str().unwrap().contains("craft-print PDF spooler"));
    }
}

/// Exercise the same OS PDF renderer as the Windows spooler, without constructing a print
/// job, creating a queue or needing a printer. This runs in the desktop-print CI matrix.
#[cfg(windows)]
#[test]
fn windows_pdf_renderer_accepts_print_pages_without_a_printer() {
    const RENDER: &str = r#"
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Runtime.WindowsRuntime
$null = [Windows.Data.Pdf.PdfDocument, Windows.Data.Pdf, ContentType = WindowsRuntime]
$null = [Windows.Storage.StorageFile, Windows.Storage, ContentType = WindowsRuntime]
$genericAwait = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
    $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncOperation`1'
} | Select-Object -First 1
function ResultOf($operation, $resultType) {
    $task = $genericAwait.MakeGenericMethod($resultType).Invoke($null, @($operation))
    $task.Wait()
    $task.Result
}
$file = ResultOf ([Windows.Storage.StorageFile]::GetFileFromPathAsync($env:PHOTOCRAFT_TEST_PRINT_PDF)) ([Windows.Storage.StorageFile])
$pdf = ResultOf ([Windows.Data.Pdf.PdfDocument]::LoadFromFileAsync($file)) ([Windows.Data.Pdf.PdfDocument])
if ($pdf.PageCount -ne 1) { throw 'expected one print page' }
$page = $pdf.GetPage(0)
$memory = [System.IO.MemoryStream]::new()
try {
    $stream = [System.IO.WindowsRuntimeStreamExtensions]::AsRandomAccessStream($memory)
    $render = $page.RenderToStreamAsync($stream)
    $awaitAction = [System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {
        $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.GetParameters()[0].ParameterType.Name -eq 'IAsyncAction'
    } | Select-Object -First 1
    $task = $awaitAction.Invoke($null, @($render))
    $task.Wait()
    if ($memory.Length -le 0) { throw 'the OS renderer produced no image' }
    @($page.Size.Width, $page.Size.Height) | ConvertTo-Json -Compress
} finally { $memory.Dispose(); $page.Dispose() }
"#;
    let dir = tmp("windows-native-render");
    for mode in ["rgb", "gray", "cmyk"] {
        let mut s = session(mode, 16);
        let output = std::path::Path::new(&dir).join(format!("{mode}.pdf"));
        s.execute("file.print", json!({"output": output, "send": false})).unwrap();
        let system = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let powershell = std::path::Path::new(&system).join("System32/WindowsPowerShell/v1.0/powershell.exe");
        let result = std::process::Command::new(powershell)
            .args(["-NoProfile", "-NonInteractive", "-Command", RENDER])
            .env("PHOTOCRAFT_TEST_PRINT_PDF", &output)
            .output()
            .unwrap();
        assert!(result.status.success(), "{mode}: {}", String::from_utf8_lossy(&result.stderr));
        let dims: Vec<f64> = serde_json::from_slice(&result.stdout).unwrap();
        // WinRT uses 96-DPI DIPs, while the PDF MediaBox uses 72-point inches.
        assert_eq!(dims, vec![816.0, 1056.0]);
    }
    let _ = std::fs::remove_dir_all(dir);
}
