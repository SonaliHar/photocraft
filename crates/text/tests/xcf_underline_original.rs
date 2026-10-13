use sha2::{Digest, Sha256};
fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
#[test]
#[ignore = "private independently authored public-GIMP underline controls"]
fn actual_underline_markup_runs_font_and_native_cache_are_exact() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-underline-evidence");
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let mut count = 0;
    for row in observations["rows"].as_array().unwrap() {
        for state in row["states"].as_array().unwrap() {
            let bytes = std::fs::read(root.join(state["xcf_file"].as_str().unwrap())).unwrap();
            assert_eq!(sha(&bytes), state["source_sha256"].as_str().unwrap());
            let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            assert_eq!(file.version, 19);
            let layer = file.layers.iter().find(|l| l.text.is_some()).unwrap();
            let text = layer.text.as_ref().unwrap();
            assert!(!layer.text_source_modified());
            assert_eq!(layer.text_flags, 1);
            assert_eq!(text.content, "HEFT");
            assert_eq!(text.markup.as_deref(), state["state"]["markup"].as_str());
            assert_eq!(text.font.as_ref().unwrap().hash.as_deref(), Some("40d692fce188e4471e2b3cba937be967878f631ad3ebbbdcd587687c7ebe0c82"));
            assert_eq!(text.parasite_flags, 1);
            assert_eq!(layer.offset, (8, 4));
            assert_eq!(layer.name, state["state"]["name"].as_str().unwrap());
            assert_eq!(layer.plane.as_ref().unwrap().bytes, std::fs::read(root.join(state["state"]["native"]["file"].as_str().unwrap())).unwrap());
            let mut cursor = 0;
            let mut underlined = Vec::new();
            for run in &text.runs {
                assert_eq!(run.start, cursor);
                assert!(run.end > run.start && run.end <= 4);
                let mut underline = false;
                for id in &run.style_ids {
                    let style = text.styles.get(*id).unwrap();
                    if style.tag == "u" {
                        underline = true;
                    }
                    for (key, value) in &style.attributes {
                        if key == "underline" {
                            assert!(matches!(value.as_str(), "single" | "none"));
                            underline = value == "single";
                        }
                    }
                }
                if underline {
                    underlined.push((run.start, run.end));
                }
                cursor = run.end;
            }
            assert_eq!(cursor, 4);
            let name = row["case"].as_str().unwrap();
            let source = state["phase"] == "source";
            let expected = if name.ends_with("whole") {
                if source { vec![(0, 4)] } else { vec![(0, 2)] }
            } else if name.ends_with("middle") {
                if source { vec![(1, 3)] } else { vec![(2, 4)] }
            } else if name.ends_with("single-attribute") {
                if source { vec![(0, 4)] } else { vec![] }
            } else {
                if source { vec![(0, 2)] } else { vec![(2, 4)] }
            };
            assert_eq!(underlined, expected, "{name}");
            let bold = text.styles.iter().any(|s| s.tag == "b");
            assert_eq!(bold, row["negative"] == true);
            assert_eq!(sha(&std::fs::read(root.join(state["xcf_file"].as_str().unwrap())).unwrap()), sha(&bytes));
            count += 1;
        }
    }
    assert_eq!(count, 32);
}
#[test]
#[ignore = "private independently authored public-GIMP underline controls"]
fn actual_underline_geometry_renderer_boundary() {
    use photocraft_raster::Interrupt;
    use photocraft_text::xcf::{self, underline};
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../photocraft-corpus/xcf/full/text-underline-evidence");
    let observations: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("observations.json")).unwrap()).unwrap();
    let font = xcf::ExactFont::bundled_inter(&Interrupt::NONE).unwrap();
    let mut count = 0;
    let mut failures = Vec::new();
    let mut rejected = 0;
    for row in observations["rows"].as_array().unwrap().iter().filter(|r| r["negative"] == false) {
        for state in row["states"].as_array().unwrap() {
            let bytes = std::fs::read(root.join(state["xcf_file"].as_str().unwrap())).unwrap();
            assert_eq!(sha(&bytes), state["source_sha256"].as_str().unwrap());
            let file = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
            let layer = &file.layers[0];
            let text = layer.text.as_ref().unwrap();
            let request = xcf::QualifiedUnderlinedText {
                request: xcf::ScaledText { content: &text.content, origin: [8, 4], size_px: row["size_px"].as_f64().unwrap() as f32 },
                antialias: if row["antialias"] == true { xcf::RectilinearAntialias::AreaCoverage } else { xcf::RectilinearAntialias::None },
                runs: underline::parse_underline_markup(&text.content, text.markup.as_deref().unwrap(), &Interrupt::NONE).unwrap(),
            };
            if row["antialias"] == false && row["size_px"] == 16 {
                assert!(
                    underline::prepare_underlined_with_charge(&font, request, 8 * 1024 * 1024, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).is_err()
                );
                rejected += 1;
                continue;
            }
            let prepared =
                underline::prepare_underlined_with_charge(&font, request, 8 * 1024 * 1024, &Interrupt::NONE, &mut |_| Ok(()), &mut |_| Ok(())).unwrap();
            let render = prepared.render_mode_with_charge(request.antialias, 8 * 1024 * 1024, &Interrupt::NONE, &mut |_| Ok(())).unwrap();
            let rect = render.rect;
            let pixels = render.surface.try_read_interleaved(rect).unwrap();
            let mut actual = vec![0u8; 128 * 64 * 4];
            for y in rect.y0..rect.y1 {
                for x in rect.x0..rect.x1 {
                    if (0..128).contains(&x) && (0..64).contains(&y) {
                        let src = usize::try_from(((y - rect.y0) * rect.width() as i32 + x - rect.x0) * 4).unwrap();
                        let dst = usize::try_from((y * 128 + x) * 4).unwrap();
                        actual[dst..dst + 4].copy_from_slice(&pixels[src..src + 4]);
                    }
                }
            }
            let expected = std::fs::read(root.join(state["state"]["projection"]["file"].as_str().unwrap())).unwrap();
            let max = actual.iter().zip(&expected).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
            println!("{} {max}/255", state["xcf_file"]);
            if max > 2 {
                let differences: Vec<_> = actual
                    .as_chunks::<4>().0.iter()
                    .zip(expected.as_chunks::<4>().0.iter())
                    .enumerate()
                    .filter(|(_, (a, b))| a != b)
                    .take(16)
                    .map(|(i, (a, b))| serde_json::json!({"x":i%128,"y":i/128,"actual":a,"expected":b}))
                    .collect();
                failures.push(serde_json::json!({"file":state["xcf_file"],"max":max,"differences":differences}));
            }
            count += 1;
        }
    }
    assert_eq!((count, rejected), (18, 6));
    std::fs::write(root.join("../underline-qualified-kernel-residuals.json"), serde_json::to_vec_pretty(&failures).unwrap()).unwrap();
    assert!(failures.is_empty());
}
