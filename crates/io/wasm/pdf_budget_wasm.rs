//! The PDF image budget as a wasm32 run (no browser, no bindings), built as a binary so it needs
//! none of the crate's dev-dependencies: a PDF whose image declares far more pixels than it holds
//! is refused before any decoder runs. On wasm32-unknown-unknown an attempt to decode it would
//! abort the module (an allocation past linear memory), so a normal return is the proof. Natively
//! it is the same check. Build and run:
//!
//! ```text
//! cargo build -p photocraft-io --bin pdf_budget_wasm --features wasm-budget-check --target wasm32-unknown-unknown
//! node crates/io/wasm/run_wasm_test.js target/wasm32-unknown-unknown/debug/pdf_budget_wasm.wasm
//! ```

use photocraft_io::pdf_import::{self, PdfOpenOptions};
use photocraft_raster::Interrupt;

/// A one-page PDF drawing one image XObject with the given dictionary and `data`.
fn one_image_pdf(dict: &str, data: &[u8]) -> Vec<u8> {
    let content = b"q 200 0 0 200 0 0 cm /X0 Do Q";
    let objs: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 200 200] /Resources << /XObject << /X0 5 0 R >> >> /Contents 4 0 R >>".to_vec(),
        stream("", content),
        stream(dict, data),
    ];
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objs.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", objs.len() + 1).as_bytes());
    for o in offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", objs.len() + 1).as_bytes());
    out
}

fn stream(extra: &str, data: &[u8]) -> Vec<u8> {
    let mut o = format!("<< /Length {} {extra} >>\nstream\n", data.len()).into_bytes();
    o.extend_from_slice(data);
    o.extend_from_slice(b"\nendstream");
    o
}

/// Every check that must hold; a failed one panics (on wasm32: traps), so the run says so.
fn main() {
    // 40000 × 40000 px declared (1.6 GP, 4.8 GB of RGB), 4 bytes of data.
    let big = one_image_pdf("/Type /XObject /Subtype /Image /Width 40000 /Height 40000 /ColorSpace /DeviceRGB /BitsPerComponent 8", &[0, 0, 0, 0]);
    let e = pdf_import::import_pdf("big.pdf", &big, &PdfOpenOptions::default(), &Interrupt::NONE).expect_err("the page is refused").to_string();
    assert!(e.contains("40000 × 40000"), "{e}");
    assert!(pdf_import::thumbnail(&big, 1, 32, None).is_err(), "the thumbnail is refused");
    assert!(pdf_import::import_image("big.pdf", &big, 1, None, None, &Interrupt::NONE).is_err(), "Images mode is refused");
    assert!(pdf_import::image_thumbnail(&big, 1, 32, None).is_err(), "the image thumbnail is refused");
    // A tiny image with the same shape opens as that image.
    let small = one_image_pdf("/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8", &[10, 20, 30, 40]);
    let r = pdf_import::import_pdf("small.pdf", &small, &PdfOpenOptions::default(), &Interrupt::NONE).expect("a small image opens");
    assert_eq!((r.document.size.width, r.document.size.height), (2, 2));
    assert!(pdf_import::thumbnail(&small, 1, 32, None).is_ok());
    println!("pdf image budget: ok");
}
