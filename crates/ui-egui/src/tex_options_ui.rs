//! Riot texture save options, using the normal transactional save path.
use crate::PhotocraftApp;
use photocraft_codecs::TexCompression;

pub struct Prompt {
    doc: photocraft_doc::DocId,
    pub path: String,
    pub compression: TexCompression,
    pub mipmaps: bool,
    pub copy: bool,
    record: &'static str,
}

pub fn is_tex_path(path: &str) -> bool {
    photocraft_engine::file_cmds::extension(path).as_deref() == Some("tex")
}

pub fn park(app: &mut PhotocraftApp, path: String, copy: bool, record: &'static str) -> Result<(), String> {
    if app.save_options_open() {
        return Err("Answer the open save options first".into());
    }
    let doc = app.active_doc_id()?;
    let settings = app.save_settings();
    app.tex_options = Some(Prompt { doc, path, compression: settings.tex_compression, mipmaps: settings.tex_mipmaps, copy, record });
    Ok(())
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = &mut app.tex_options else { return };
    let (mut ok, mut cancel) = (false, false);
    let modal = egui::Modal::new(egui::Id::new("riot-tex-options")).show(ctx, |ui| {
        ui.set_max_width(390.0);
        ui.label(egui::RichText::new("Riot TEX").font(crate::theme::semibold(15.0)));
        ui.add_space(8.0);
        crate::widgets::hairline(ui);
        for c in TexCompression::ALL {
            ui.radio_value(&mut p.compression, c, c.name());
        }
        ui.add_space(8.0);
        ui.checkbox(&mut p.mipmaps, tl!("Generate mipmaps"));
        ui.label(tl!("BC1 / BC3 / BC5 / BC7 require dimensions divisible by 4."));
        if p.compression == TexCompression::Bc5 {
            ui.label(tl!("BC5 stores only the red and green channels."));
        }
        ui.label(tl!("Mipmaps are regenerated from the full-size image."));
        ui.add_space(10.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ok = crate::widgets::primary_button(ui, tl!("OK"), 84.0).clicked();
            cancel = crate::widgets::secondary_button(ui, tl!("Cancel"), 84.0).clicked();
        });
    });
    cancel |= modal.should_close();
    if ok || ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter)) {
        let Some(p) = app.tex_options.take() else { return };
        let step = serde_json::json!({"path":p.path,"texCompression":p.compression.code(),"texMipmaps":p.mipmaps});
        let written = app.with_document(p.doc, |app| {
            let settings = crate::ExportSettings { tex_compression: p.compression, tex_mipmaps: p.mipmaps, ..app.save_settings() };
            app.write_document(p.path, &settings, p.copy)
        });
        if written.is_ok() {
            app.session.journal.push((p.record.into(), step));
        }
        match written {
            Err(e) => {
                app.ui.status = format!("TEX: {e}");
                app.ui.status_error = true;
            }
            Ok(Some(_)) if !p.copy => crate::discard_ui::saved_document(app, ctx, p.doc),
            Ok(_) => {}
        }
    } else if cancel {
        app.tex_options = None;
        app.ui.status = tl!("Save cancelled").into();
        app.ui.status_error = false;
    }
}

/// Keep settings only after a successful non-copy save, on the originating document.
pub(crate) fn remember(app: &mut PhotocraftApp, doc: photocraft_doc::DocId, path: &str, settings: &crate::ExportSettings, copy: bool) {
    if copy || !is_tex_path(path) {
        return;
    }
    let _ = app.with_document(doc, |app| {
        if let Some(st) = app.session.active_mut() {
            let d = std::sync::Arc::make_mut(&mut st.doc);
            d.file_encoding.tex_compression = Some(settings.tex_compression.code());
            d.file_encoding.tex_mipmaps = Some(settings.tex_mipmaps);
        }
        Ok(())
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn texture_names() {
        assert!(is_tex_path("C:\\textures\\skin.TEX"));
        assert!(!is_tex_path("skin.tex.png"));
        assert!(!is_tex_path("tex"));
    }
}

#[cfg(test)]
mod flow_tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    use serde_json::json;
    use std::{cell::RefCell, rc::Rc};

    #[test]
    fn riot_tex_options_write_selected_encoding_cancel_and_replay() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width":8,"height":8})).unwrap();
        let writes: Rc<RefCell<Vec<Vec<u8>>>> = Rc::default();
        let sink = writes.clone();
        app.services.export = Some(Box::new(|doc, path, settings| {
            let mut opts = photocraft_io::ExportOptions::default();
            opts.encode.tex_compression = settings.tex_compression;
            opts.encode.tex_mipmaps = settings.tex_mipmaps;
            photocraft_io::export(doc, path, &opts).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
        }));
        app.services.write = Some(Box::new(move |_, bytes| {
            sink.borrow_mut().push(bytes.to_vec());
            Ok(())
        }));
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.state_mut().save_as(Some("skin.tex".into())).unwrap();
        h.run_steps(2);
        assert!(writes.borrow().is_empty());
        assert!(h.state_mut().save_as(Some("other.tex".into())).is_err());
        h.get_by_label("BC7 (RGBA)").click();
        h.run_steps(2);
        h.get_by_label("OK").click();
        h.run_steps(3);
        assert_eq!(writes.borrow().len(), 1);
        assert_eq!(photocraft_codecs::tex_info(&writes.borrow()[0]).unwrap().compression, TexCompression::Bc7);
        assert_eq!(h.state().session.active().unwrap().doc.file_encoding.tex_compression, Some(13));
        h.state_mut().save_as(Some("cancelled.tex".into())).unwrap();
        h.run_steps(2);
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        assert!(h.state().tex_options.is_none());
        assert_eq!(writes.borrow().len(), 1);
        h.state_mut().replay_save("file.saveAs", &json!({"path":"replay.tex","texCompression":12,"texMipmaps":false})).unwrap();
        let info = photocraft_codecs::tex_info(&writes.borrow()[1]).unwrap();
        assert_eq!(info.compression, TexCompression::Bc3);
        assert!(!info.mipmaps);
        let original_path = h.state().session.active().unwrap().path.clone();
        let original_encoding = h.state().session.active().unwrap().doc.file_encoding;
        h.state_mut().replay_save("file.saveACopy", &json!({"path":"copy.tex","texCompression":13,"texMipmaps":true})).unwrap();
        assert_eq!(h.state().session.active().unwrap().path, original_path);
        assert_eq!(h.state().session.active().unwrap().doc.file_encoding, original_encoding);
        assert_eq!(photocraft_codecs::tex_info(&writes.borrow()[2]).unwrap().compression, TexCompression::Bc7);
        assert_eq!(h.state().session.journal.last().unwrap().0, "file.saveACopy");
        for bad in [json!({"texCompression":999}), json!({"texCompression":"bc7"}), json!({"texMipmaps":1})] {
            assert!(photocraft_engine::file_cmds::tex_params(&bad, "file.saveAs").is_err());
        }
    }
}
