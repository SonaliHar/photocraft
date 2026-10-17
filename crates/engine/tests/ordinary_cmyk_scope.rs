//! Public ordinary CMYK stroke regression; no imported/profile-domain qualification.
use photocraft_engine::{Session, brush_cmds::LiveStroke};
use photocraft_geom::Rect;
use photocraft_paint::StrokePoint;
use photocraft_raster::Surface;
use serde_json::{Value, json};

fn surface(s: &Session) -> Surface {
    let active = s.active().unwrap();
    active.doc.layer(active.active_layer.unwrap()).unwrap().surface().unwrap().clone()
}
fn session(depth: u32, profile: Option<&std::sync::Arc<Vec<u8>>>) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":32,"height":32,"mode":"cmyk","depth":depth,"background":"white"})).unwrap();
    assert!(s.active().unwrap().doc.icc_profile.is_none(), "original untagged default role");
    if let Some(profile) = profile {
        s.edit("icc", |doc, _| {
            doc.icc_profile = Some(profile.clone());
            Ok(())
        })
        .unwrap();
    }
    s
}

#[test]
fn assigned_and_default_cmyk_live_and_registered_strokes_preserve_native_history() {
    // Exact original synthetic uncoated-like profile: distinct from default coated CMYK.
    let profile = photocraft_cms::synth::cmyk_profile(&photocraft_cms::synth::CmykParams {
        description: "Stroke Test Uncoated".into(),
        tvi: [0.26, 0.26, 0.26, 0.3],
        grid_a2b: 5,
        grid_b2a: 9,
        ..Default::default()
    })
    .to_bytes();
    let bounds = Rect::new(0, 0, 32, 32);
    for depth in [8, 16, 32] {
        let mut inks = Vec::new();
        for tagged in [true, false] {
            let mut s = session(depth, tagged.then_some(&profile));
            let before = s.active().unwrap().doc.clone();
            let revision = s.active().unwrap().revision;
            let steps = s.active().unwrap().history.past_len();
            let initial = surface(&s);
            let params = json!({"points":[[4,16],[28,16]],"size":8,"hardness":1,"opacity":1,"flow":1,"color":"#cc3366","smoothing":0});
            let mut begin = params.clone();
            begin["points"] = json!([[4, 16]]);
            let mut live = LiveStroke::begin(&s, &begin).unwrap();
            live.push(&[StrokePoint::new(28.0, 16.0, 1.0)]).unwrap();
            let shown = live.doc.layer(s.active().unwrap().active_layer.unwrap()).unwrap().surface().unwrap().clone();
            let seed = live.seed;
            assert!(s.active().unwrap().doc.shares_state(&before));
            assert_eq!(s.active().unwrap().revision, revision);
            assert_eq!(s.active().unwrap().history.past_len(), steps);
            // Do not offer the preview: the registered command must independently render.
            drop(live);
            let mut commit = params;
            commit["seed"] = json!(seed);
            let reply = s.execute("paint.stroke", commit).unwrap();
            assert!(reply.get("fromLive").is_none());
            let painted = surface(&s);
            assert_eq!(painted.format(), initial.format());
            assert_eq!(painted.default_bytes(), initial.default_bytes());
            assert_eq!(painted.try_read_interleaved(bounds).unwrap(), shown.try_read_interleaved(bounds).unwrap(), "depth={depth}, tagged={tagged}");
            assert_eq!(painted, shown, "full native tiles/defaults, depth={depth}, tagged={tagged}");
            assert_ne!(painted.try_read_interleaved(bounds).unwrap(), initial.try_read_interleaved(bounds).unwrap());
            assert_eq!(s.active().unwrap().history.past_len(), steps + 1);
            assert_eq!(s.active().unwrap().revision, revision + 1);
            assert_eq!(s.active().unwrap().doc.icc_profile, before.icc_profile);
            assert_eq!(s.active().unwrap().doc.sample_encoding, before.sample_encoding);
            assert_eq!(s.active().unwrap().doc.source_color_context, before.source_color_context);
            // Original independent semantic control: custom-profile ink differs from default ink.
            inks.push(painted.read_region(Rect::new(15, 16, 17, 18)));
            let edited = s.active().unwrap().doc.clone();
            s.execute("edit.undo", Value::Null).unwrap();
            assert_eq!(s.active().unwrap().doc, before);
            assert_eq!(surface(&s), initial);
            s.execute("edit.redo", Value::Null).unwrap();
            assert_eq!(s.active().unwrap().doc, edited);
            assert_eq!(surface(&s), painted);
            let bytes = photocraft_format::save_to_bytes(&edited, &Default::default()).unwrap();
            let reopened =
                photocraft_format::load_from_bytes_with(&bytes, &photocraft_format::LoadOptions { preserve_ids: true, ..Default::default() }).unwrap();
            assert_eq!(reopened, *edited);
            let id = s.active().unwrap().active_layer.unwrap();
            assert_eq!(reopened.layer(id).unwrap().surface().unwrap(), &painted);
            assert_eq!(reopened.icc_profile, before.icc_profile);
        }
        assert_ne!(inks[0], inks[1], "depth={depth}: assigned CMYK profile must change painted ink");
    }
}
