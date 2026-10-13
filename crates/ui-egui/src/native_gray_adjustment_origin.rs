//! Runtime-only target fencing for existing Threshold, Posterize, Levels, Curves, Exposure and Brightness/Contrast dialogs.
//! This does not qualify source metadata or replace the registered engine's native checks.
use crate::PhotocraftApp;
use photocraft_doc::{DocId,LayerId};
use photocraft_engine::{WeakDocumentSnapshot,channel_cmds::ChannelTarget,mask_view_cmds::LayerMaskView};
use serde_json::{Map,Value};
#[derive(Clone,Debug)]
pub(crate) struct Origin {dialog:u64,command:String,document:DocId,revision:u64,snapshot:WeakDocumentSnapshot,layer:Option<LayerId>,channel:ChannelTarget,editing_mask:Option<LayerId>,shown_mask:Option<LayerMaskView>,shell_mask:bool}
impl PartialEq for Origin {
    fn eq(&self,other:&Self)->bool {
        self.dialog==other.dialog && self.command==other.command && self.document==other.document && self.revision==other.revision
            && self.layer==other.layer && self.channel==other.channel && self.editing_mask==other.editing_mask && self.shown_mask==other.shown_mask && self.shell_mask==other.shell_mask
            && match(self.snapshot.upgrade(),other.snapshot.upgrade()) {(Some(a),Some(b))=>a.shares_state(&b),(None,None)=>true,_=>false}
    }
}
pub(crate) fn handles(command:&str)->bool {matches!(command,"image.adjustments.threshold"|"image.adjustments.posterize"|"image.adjustments.levels"|"image.adjustments.curves"|"image.adjustments.exposure"|"image.adjustments.brightnessContrast")}
fn idle(app:&PhotocraftApp)->Result<(),String> {
    let active=app.session.active().ok_or_else(||tl!("Open a document before adjusting pixels.").to_owned())?;
    if active.doc.id.0==0 || active.revision==u64::MAX || app.ui.transform.is_some() || app.session.has_jobs() || active.floating.is_some() || active.coalesce.is_some()
        || active.doc.xcf_state.as_ref().is_some_and(|state|state.floating.is_some()) {return Err(tl!("Finish pending edits before adjusting pixels.").into());}
    Ok(())
}
pub(crate) fn capture_open(app:&mut PhotocraftApp,dialog:u64,command:&str) {
    if !handles(command) {return;}
    app.ui.native_gray_adjustment_origin=None;
    if let Err(error)=idle(app) {app.ui.status=error;app.ui.status_error=true;return;}
    if let Some(active)=app.session.active() {
        app.ui.native_gray_adjustment_origin=Some(Origin {dialog,command:command.into(),document:active.doc.id,revision:active.revision,snapshot:active.doc.downgrade(),layer:active.active_layer,channel:active.channel_view.target,editing_mask:active.channel_view.editing_mask,shown_mask:active.channel_view.layer_mask,shell_mask:app.layer_mask_targeted()});
    }
}
fn levels_number(key:&str)->bool {matches!(key,"inBlack"|"inWhite"|"gamma"|"outBlack"|"outWhite")}
fn levels_channel(key:&str)->bool {matches!(key,"gray"|"red"|"green"|"blue"|"cyan"|"magenta"|"yellow"|"black"|"lightness"|"a"|"b")}
fn levels_record(value:&Value)->bool {
    value.as_object().is_some_and(|record|record.len()<=5 && record.iter().all(|(key,value)|levels_number(key) && value.as_f64().is_some_and(f64::is_finite)))
}
fn levels_settings(fields:&Map<String,Value>)->bool {
    fields.len()<=16 && fields.iter().all(|(key,value)| {
        if levels_number(key) {value.as_f64().is_some_and(f64::is_finite)}
        else if levels_channel(key) {levels_record(value)}
        else {false}
    })
}
fn curves_values(value:&Value)->bool {
    let Some(entries)=value.as_array() else {return false};
    if entries.first().is_some_and(Value::is_number) {
        entries.len()==256 && entries.iter().all(|value|value.as_f64().is_some_and(f64::is_finite))
    } else {
        (2..=64).contains(&entries.len()) && entries.iter().all(|value|value.as_array().is_some_and(|pair|pair.len()==2 && pair.iter().all(|value|value.as_f64().is_some_and(|number|number.is_finite() && (-1e-6..=255.0+1e-6).contains(&number)))))
    }
}
fn curves_settings(fields:&Map<String,Value>)->bool {
    fields.len()<=14 && fields.iter().all(|(key,value)|match key.as_str() {
        "pencil"=>value.is_boolean(),
        "smooth"=>value.as_f64().is_some_and(f64::is_finite),
        "points"=>curves_values(value),
        key if levels_channel(key)=>curves_values(value),
        _=>false,
    })
}
/// Borrowed validation before generic parameter cloning or remembered preferences.
pub(crate) fn validate(app:&PhotocraftApp,dialog:u64,fields:&Map<String,Value>)->Result<(),String> {
    let command=fields.get("__command").and_then(Value::as_str).ok_or_else(||tl!("Reopen the adjustment dialog before applying.").to_owned())?;
    if !handles(command) {return Ok(());}
    idle(app)?;
    let stale=||tl!("The adjustment target changed. Reopen the dialog.").to_owned();
    let origin=app.ui.native_gray_adjustment_origin.as_ref().ok_or_else(stale)?;
    let active=app.session.active().ok_or_else(stale)?;
    let snapshot=origin.snapshot.upgrade().ok_or_else(stale)?;
    if origin.dialog!=dialog || origin.command!=command || active.doc.id!=origin.document || active.revision!=origin.revision || !active.doc.shares_state(&snapshot)
        || active.active_layer!=origin.layer || active.channel_view.target!=origin.channel || active.channel_view.editing_mask!=origin.editing_mask || active.channel_view.layer_mask!=origin.shown_mask || app.layer_mask_targeted()!=origin.shell_mask {return Err(stale());}
    let invalid=||tl!("Check the adjustment settings and try again.").to_owned();
    let levels=command=="image.adjustments.levels";
    let curves=command=="image.adjustments.curves";
    let exposure=command=="image.adjustments.exposure";
    let brightness_contrast=command=="image.adjustments.brightnessContrast";
    if fields.len()>32 || fields.iter().any(|(key,value)|key.len()>128 || (!key.starts_with("__") && (!(matches!(key.as_str(),"opacity"|"target"|"layer"|"documentId"|"expectedRevision") || if levels {levels_number(key)||levels_channel(key)} else if curves {matches!(key.as_str(),"points"|"pencil"|"smooth")||levels_channel(key)} else if exposure {matches!(key.as_str(),"exposure"|"offset"|"gamma")} else if brightness_contrast {matches!(key.as_str(),"brightness"|"contrast"|"legacy")} else {matches!(key.as_str(),"level"|"levels")}) || matches!(value,Value::String(text) if text.len()>128)))) {return Err(invalid());}
    if levels {
        for (key,value) in fields {
            if (levels_number(key) && value.as_f64().is_none_or(|n|!n.is_finite())) || (levels_channel(key) && !levels_record(value)) {return Err(invalid());}
        }
        // Eyedropper state uses the same bounded records; it does not become engine authority.
        if fields.get("__curvePickerBase").is_some_and(|value|value.as_object().is_none_or(|base|!levels_settings(base))) {return Err(invalid());}
    }
    if curves {
        for (key,value) in fields {
            if ((key=="points" || levels_channel(key)) && !curves_values(value))
                || (key=="pencil" && !value.is_boolean())
                || (key=="smooth" && value.as_f64().is_none_or(|number|!number.is_finite())) {return Err(invalid());}
        }
        if fields.get("__curvePickerBase").is_some_and(|value|value.as_object().is_none_or(|base|!curves_settings(base))) {return Err(invalid());}
    }
    if exposure {
        for key in ["exposure","offset","gamma"] {
            if fields.get(key).is_some_and(|value|value.as_f64().is_none_or(|number|!number.is_finite())) {return Err(invalid());}
        }
    }
    if brightness_contrast {
        for key in ["brightness","contrast"] {
            if fields.get(key).is_some_and(|value|value.as_f64().is_none_or(|number|!number.is_finite())) {return Err(invalid());}
        }
        if fields.get("legacy").is_some_and(|value|!value.is_boolean()) {return Err(invalid());}
    }
    for key in ["level","levels","opacity"] {if fields.get(key).is_some_and(|value|value.as_f64().is_none_or(|n|!n.is_finite())) {return Err(invalid());}}
    if fields.get("__kind").is_some_and(|value|value.as_str()!=command.strip_prefix("image.adjustments.")) {return Err(invalid());}
    if fields.get("__smartFilter").is_some_and(|value|value.as_object().is_none_or(|object|object.len()!=2 || object.get("layer").and_then(Value::as_u64).is_none() || object.get("index").and_then(Value::as_u64).is_none())) {return Err(invalid());}
    if fields.get("target").is_some_and(|value|match value {
        Value::String(text)=>!matches!(text.as_str(),"mask"|"pixels"|"quickMask"),
        Value::Object(object)=>object.len()!=1 || object.get("channel").and_then(Value::as_u64).is_none(),
        _=>true,
    }) || fields.get("layer").is_some_and(|value|value.as_u64().is_none_or(|id|id==0)) {
        return Err(invalid());
    }
    for key in ["documentId","expectedRevision"] {
        if fields.get(key).is_some_and(|value|value.as_str().is_none_or(|text|text.is_empty() || text.len()>20 || !text.bytes().all(|b|b.is_ascii_digit()) || (text.len()>1 && text.starts_with('0')) || text.parse::<u64>().is_err())) {return Err(invalid());}
    }
    if fields.contains_key("documentId")!=fields.contains_key("expectedRevision") {return Err(invalid());}
    Ok(())
}
/// Preserve ordinary dialog/Smart Filter parameters. Only the native wide lane adds selectors.
pub(crate) fn prepare(app:&PhotocraftApp,dialog:u64,fields:&Map<String,Value>,mut params:Value)->Result<Value,String> {
    validate(app,dialog,fields)?;
    let command=fields.get("__command").and_then(Value::as_str).ok_or_else(||tl!("Reopen the adjustment dialog before applying.").to_owned())?;
    if !handles(command) || fields.contains_key("__smartFilter") {return Ok(params);}
    let origin=app.ui.native_gray_adjustment_origin.as_ref().ok_or_else(||tl!("Reopen the adjustment dialog before applying.").to_owned())?;
    let active=app.session.active().ok_or_else(||tl!("Open a document before adjusting pixels.").to_owned())?;
    // Follow the existing shell mask injection and registered native target precedence.
    let mask=||origin.layer.and_then(|id|active.doc.layer(id)).and_then(|layer|layer.mask.as_ref()).map(|mask|mask.surface.as_ref());
    let plain_pixels=match params.get("target") {
        Some(Value::String(target))=>target=="pixels",
        None=>!origin.shell_mask && origin.channel==ChannelTarget::Composite && active.doc.quick_mask.is_none() && photocraft_engine::mask_view_cmds::current(active).is_none(),
        _=>false,
    };
    let pixels=||params.get("layer").and_then(Value::as_u64).map(LayerId).or(origin.layer)
        .and_then(|id|active.doc.layer(id)).and_then(|layer|match &layer.content {
            photocraft_doc::LayerContent::Raster(surface) if matches!(surface.format().mode,photocraft_doc::ColorMode::Rgb|photocraft_doc::ColorMode::Grayscale)=>Some(surface),
            _=>None,
        });
    let surface=match params.get("target") {
        Some(Value::String(target)) if target=="mask"=>mask(),
        Some(Value::String(target)) if target=="pixels"=>pixels(),
        Some(Value::String(target)) if target=="quickMask"=>active.doc.quick_mask.as_ref().map(|channel|&channel.surface),
        Some(Value::Object(object))=>object.get("channel").and_then(Value::as_u64).and_then(|index|usize::try_from(index).ok()).and_then(|index|active.doc.channels.get(index)).map(|channel|&channel.surface),
        Some(_)=>None,
        None if origin.shell_mask=>mask(),
        None=>match origin.channel {
            ChannelTarget::Alpha(index)=>active.doc.channels.get(index).map(|channel|&channel.surface),
            ChannelTarget::LayerMask=>mask(),
            ChannelTarget::Composite if active.doc.quick_mask.is_some()=>active.doc.quick_mask.as_ref().map(|channel|&channel.surface),
            ChannelTarget::Composite if photocraft_engine::mask_view_cmds::current(active).is_some()=>mask(),
            ChannelTarget::Composite=>pixels(),
            _=>None,
        },
    };
    if surface.is_some_and(|surface|matches!(surface.format().sample,photocraft_doc::SampleType::U32|photocraft_doc::SampleType::F64)
        && (command!="image.adjustments.brightnessContrast" || (params.get("legacy").and_then(Value::as_bool)==Some(true) && (plain_pixels || (surface.format().mode==photocraft_doc::ColorMode::Grayscale && !surface.format().alpha))))) {
        let object=params.as_object_mut().filter(|object|object.len()<=if matches!(command,"image.adjustments.levels"|"image.adjustments.curves") {18} else {8}).ok_or_else(||tl!("Check the adjustment settings and try again.").to_owned())?;
        object.insert("documentId".into(),Value::String(origin.document.0.to_string()));
        object.insert("expectedRevision".into(),Value::String(origin.revision.to_string()));
    }
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn threshold_target_switch_and_restored_posterize_fields_have_no_authority() {
        let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());
        app.session.execute("file.new",json!({"width":4,"height":4})).unwrap();
        for (command,key) in [("image.adjustments.threshold","level"),("image.adjustments.posterize","levels")] {
            capture_open(&mut app,9,command);
            let fields=json!({"__command":command,key:3}).as_object().unwrap().clone();
            assert!(validate(&app,9,&fields).is_ok());
            let before=app.session.active().unwrap().doc.clone();
            app.session.active_mut().unwrap().channel_view.target=ChannelTarget::Color(0);
            assert!(validate(&app,9,&fields).is_err());
            assert!(app.session.active().unwrap().doc.shares_state(&before));
            app.session.active_mut().unwrap().channel_view.target=ChannelTarget::Composite;
            app.ui.native_gray_adjustment_origin=None;
            assert!(prepare(&app,9,&fields,json!({key:3})).is_err());
        }
        assert!(!handles("image.adjustments.invert"));
    }
    #[test]
    fn levels_plain_wide_raster_carries_pair_and_refuses_changed_origin() {
        for mode in [photocraft_doc::ColorMode::Rgb,photocraft_doc::ColorMode::Grayscale] {
            for sample in [photocraft_doc::SampleType::U32,photocraft_doc::SampleType::F64] {
                let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());
                app.session.execute("file.new",json!({"width":2,"height":2})).unwrap();
                let layer=app.session.active().unwrap().active_layer.unwrap();
                let doc=app.session.active_mut().unwrap().doc.try_resident_mut().unwrap();
                doc.mode=mode;doc.depth=sample;
                doc.layer_mut(layer).unwrap().content=photocraft_doc::LayerContent::Raster(photocraft_raster::Surface::new(photocraft_doc::PixelFormat {mode,sample,alpha:true}));
                for kind in ["levels","curves","exposure"] {
                let command=format!("image.adjustments.{kind}");
                let params=photocraft_engine::adjust_params::to_params(&photocraft_engine::adjust_params::default_for(kind,mode).unwrap());
                let mut fields=params.as_object().unwrap().clone();fields.insert("__command".into(),json!(command));
                capture_open(&mut app,12,&command);
                let prepared=prepare(&app,12,&fields,params.clone()).unwrap();
                assert_eq!(prepared["documentId"],json!(app.session.active().unwrap().doc.id.0.to_string()));
                assert_eq!(prepared["expectedRevision"],json!(app.session.active().unwrap().revision.to_string()));
                for (key,value) in params.as_object().unwrap() {assert_eq!(prepared.get(key),Some(value));}
                app.session.active_mut().unwrap().revision+=1;
                assert!(prepare(&app,12,&fields,params).is_err());
                }
            }
        }
    }
    #[test]
    fn levels_records_are_bounded_and_ordinary_smart_filter_wire_is_preserved() {
        let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());
        app.session.execute("file.new",json!({"width":2,"height":2})).unwrap();
        for mode in [photocraft_doc::ColorMode::Rgb,photocraft_doc::ColorMode::Cmyk,photocraft_doc::ColorMode::Lab] {
            let params=photocraft_engine::adjust_params::to_params(&photocraft_engine::adjust_params::default_for("levels",mode).unwrap());
            let mut fields=params.as_object().unwrap().clone();fields.insert("__command".into(),json!("image.adjustments.levels"));
            fields.insert("__smartFilter".into(),json!({"layer":1,"index":0}));
            capture_open(&mut app,13,"image.adjustments.levels");
            assert_eq!(prepare(&app,13,&fields,params.clone()).unwrap(),params);
            fields.insert("red".into(),json!({"gamma":[1,2]}));
            assert!(validate(&app,13,&fields).is_err());
            fields.insert("red".into(),json!({"unexpected":1}));
            assert!(validate(&app,13,&fields).is_err());
        }
        assert!(!levels_record(&json!({"gamma":[1,2,3]})));
    }

    #[test]
    fn curves_shapes_bound_pairs_maps_and_keep_ordinary_smart_filter_fields() {
        assert!(curves_values(&json!([[0,0],[255,255]])));
        assert!(curves_values(&json!((0..256).collect::<Vec<_>>())));
        assert!(!curves_values(&json!(vec![json!([0,0]);65])));
        assert!(!curves_values(&json!(vec![json!(1);257])));
        assert!(!curves_values(&json!([[0,0],[255,{"nested":1}]])));
        let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());
        app.session.execute("file.new",json!({"width":2,"height":2})).unwrap();
        for mode in [photocraft_doc::ColorMode::Rgb,photocraft_doc::ColorMode::Cmyk,photocraft_doc::ColorMode::Lab] {
            let params=photocraft_engine::adjust_params::to_params(&photocraft_engine::adjust_params::default_for("curves",mode).unwrap());
            let mut fields=params.as_object().unwrap().clone();fields.insert("__command".into(),json!("image.adjustments.curves"));
            fields.insert("__smartFilter".into(),json!({"layer":1,"index":0}));
            fields.insert("__curvePickerBase".into(),params.clone());
            capture_open(&mut app,14,"image.adjustments.curves");
            assert_eq!(prepare(&app,14,&fields,params).unwrap(),filter_params_without_private(&fields));
            fields.insert("pencil".into(),json!("yes"));
            assert!(validate(&app,14,&fields).is_err());
        }
    }
    fn filter_params_without_private(fields:&Map<String,Value>)->Value {
        Value::Object(fields.iter().filter(|(key,_)|!key.starts_with("__")).map(|(key,value)|(key.clone(),value.clone())).collect())
    }

    #[test]
    fn exposure_scalars_are_bounded_and_smart_filter_params_stay_unchanged() {
        let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());
        app.session.execute("file.new",json!({"width":2,"height":2})).unwrap();
        let params=json!({"exposure":1.25,"offset":-0.1,"gamma":1.1});
        let mut fields=params.as_object().unwrap().clone();fields.insert("__command".into(),json!("image.adjustments.exposure"));
        fields.insert("__smartFilter".into(),json!({"layer":1,"index":0}));
        capture_open(&mut app,15,"image.adjustments.exposure");
        assert_eq!(prepare(&app,15,&fields,params.clone()).unwrap(),params);
        for key in ["exposure","offset","gamma"] {
            let mut malformed=fields.clone();malformed.insert(key.into(),json!([1,2]));
            assert!(validate(&app,15,&malformed).is_err());
        }
        fields.insert("unknown".into(),json!(1));assert!(validate(&app,15,&fields).is_err());
    }

    #[test]
    fn legacy_brightness_gray_mask_stamps_pair_and_narrow_raster_stays_ordinary() {
        let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());
        app.session.execute("file.new",json!({"width":2,"height":2})).unwrap();
        let layer=app.session.active().unwrap().active_layer.unwrap();
        let params=json!({"brightness":20,"contrast":-10,"legacy":true});
        let mut fields=params.as_object().unwrap().clone();fields.insert("__command".into(),json!("image.adjustments.brightnessContrast"));
        capture_open(&mut app,16,"image.adjustments.brightnessContrast");
        assert_eq!(prepare(&app,16,&fields,params.clone()).unwrap(),params);
        let mut mask=photocraft_doc::LayerMask::hide_all();
        mask.surface=std::sync::Arc::new(photocraft_raster::Surface::new(photocraft_doc::PixelFormat::new(photocraft_doc::ColorMode::Grayscale,photocraft_doc::SampleType::F64,false)));
        app.session.active_mut().unwrap().doc.try_resident_mut().unwrap().layer_mut(layer).unwrap().mask=Some(mask);
        app.ui.mask_target=true;capture_open(&mut app,17,"image.adjustments.brightnessContrast");
        let prepared=prepare(&app,17,&fields,params.clone()).unwrap();
        assert_eq!(prepared["documentId"],json!(app.session.active().unwrap().doc.id.0.to_string()));
        assert_eq!(prepared["legacy"],true);
        let modern=json!({"brightness":20,"contrast":-10,"legacy":false});
        assert_eq!(prepare(&app,17,&fields,modern.clone()).unwrap(),modern);
        let mut smart=fields.clone();smart.insert("__smartFilter".into(),json!({"layer":layer.0,"index":0}));
        assert_eq!(prepare(&app,17,&smart,params.clone()).unwrap(),params);
        for key in ["brightness","contrast","legacy"] {
            let mut malformed=fields.clone();malformed.insert(key.into(),json!([1,2]));assert!(validate(&app,17,&malformed).is_err());
        }
        app.session.active_mut().unwrap().revision+=1;assert!(prepare(&app,17,&fields,params).is_err());
    }

    #[test]
    fn legacy_brightness_plain_native_pixels_get_pair_without_modern_or_smart_changes() {
        for mode in [photocraft_doc::ColorMode::Rgb,photocraft_doc::ColorMode::Grayscale] {
            for sample in [photocraft_doc::SampleType::U32,photocraft_doc::SampleType::F64] {
                let mut app=PhotocraftApp::new(photocraft_engine::Session::new(),Default::default());
                app.session.execute("file.new",json!({"width":2,"height":2})).unwrap();
                let layer=app.session.active().unwrap().active_layer.unwrap();
                let doc=app.session.active_mut().unwrap().doc.try_resident_mut().unwrap();doc.mode=mode;doc.depth=sample;
                doc.layer_mut(layer).unwrap().content=photocraft_doc::LayerContent::Raster(photocraft_raster::Surface::new(photocraft_doc::PixelFormat::new(mode,sample,true)));
                let params=json!({"brightness":20,"contrast":-10,"legacy":true});
                let mut fields=params.as_object().unwrap().clone();fields.insert("__command".into(),json!("image.adjustments.brightnessContrast"));
                capture_open(&mut app,18,"image.adjustments.brightnessContrast");
                for explicit in [false,true] {
                    let mut request=params.clone();if explicit {request["target"]=json!("pixels");}
                    let prepared=prepare(&app,18,&fields,request).unwrap();
                    assert_eq!(prepared["documentId"],json!(app.session.active().unwrap().doc.id.0.to_string()));
                    assert_eq!(prepared["expectedRevision"],json!(app.session.active().unwrap().revision.to_string()));
                }
                let modern=json!({"brightness":20,"contrast":-10,"legacy":false});
                assert_eq!(prepare(&app,18,&fields,modern.clone()).unwrap(),modern);
                let mut smart=fields.clone();smart.insert("__smartFilter".into(),json!({"layer":layer.0,"index":0}));
                assert_eq!(prepare(&app,18,&smart,params.clone()).unwrap(),params);
                app.session.active_mut().unwrap().revision+=1;assert!(prepare(&app,18,&fields,params).is_err());
            }
        }
    }

}
