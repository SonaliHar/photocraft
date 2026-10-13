//! Recording-only runtime fence normalization. Journal and external command parameters
//! retain their original guards; only newly recorded reusable recipes lose frame fences.
use crate::{EngineError,Result};
use serde_json::Value;
fn relative(id:&str)->bool {
    matches!(id,"select.all"|"select.deselect"|"select.inverse"|"select.rect"|"edit.fill"|"edit.clear"|"edit.copy"|"edit.cut"|"edit.paste"
        |"edit.pasteSpecial.pasteInPlace"|"edit.pasteSpecial.pasteInto"|"edit.pasteSpecial.pasteOutside"
        |"layer.layerMask.revealAll"|"layer.layerMask.hideAll"|"layer.layerMask.revealSelection"
        |"layer.layerMask.hideSelection"|"layer.layerMask.fromTransparency"|"layer.layerMask.delete"|"layer.layerMask.apply"
        |"xcf.floating.anchor"|"xcf.floating.toLayer"|"xcf.floating.delete"
        |"image.adjustments.threshold"|"image.adjustments.posterize"|"image.adjustments.invert"|"image.adjustments.levels"|"image.adjustments.curves"|"image.adjustments.exposure"|"layer.new.layer"|"layer.duplicate"
        |"select.modify.feather"|"select.modify.expand"|"select.modify.contract"|"select.modify.border"|"select.modify.smooth")
}
fn canonical(value:&Value,nonzero:bool)->bool {
    let Some(text)=value.as_str() else {return false};
    !text.is_empty() && text.len()<=20 && text.bytes().all(|byte|byte.is_ascii_digit())
        && (text.len()==1 || !text.starts_with('0')) && text.parse::<u64>().is_ok_and(|id|!nonzero || id!=0)
}
fn bad()->EngineError {EngineError::BadParams {cmd:"actions.stop".into(),msg:"recorded command has an invalid captured document/revision pair".into()}}
pub(super) fn normalize(id:&str,mut params:Value)->Result<Value> {
    if !relative(id) {return Ok(params)}
    match (params.get("documentId"),params.get("expectedRevision")) {
        (None,None)=>return Ok(params),
        (Some(document),Some(revision)) if canonical(document,true) && canonical(revision,false)=>{},
        _=>return Err(bad()),
    }
    let object=params.as_object_mut().ok_or_else(bad)?;
    object.remove("documentId");object.remove("expectedRevision");
    Ok(params)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn recording_removes_only_canonical_runtime_pair_and_keeps_exact_target() {
        let journal=json!({"documentId":"18446744073709551615","expectedRevision":"0","layer":9007199254740993u64,"target":{"channel":2},"center":[3,4]});
        let recipe=normalize("edit.paste",journal.clone()).unwrap();
        assert_eq!(recipe,json!({"layer":9007199254740993u64,"target":{"channel":2},"center":[3,4]}));
        assert_eq!(journal["documentId"],json!("18446744073709551615"));
        assert_eq!(normalize("xcf.link.substitute",journal.clone()).unwrap(),journal);
        assert_eq!(normalize("select.inverse",json!({"documentId":"7","expectedRevision":"9"})).unwrap(),json!({}));
        assert_eq!(normalize("select.rect",json!({"x":3,"y":4,"width":5,"height":6,"ellipse":true,"documentId":"7","expectedRevision":"9"})).unwrap(),json!({"x":3,"y":4,"width":5,"height":6,"ellipse":true}));
        for command in ["edit.fill","edit.clear"] {
            assert_eq!(normalize(command,json!({"target":"mask","contents":"background","documentId":"7","expectedRevision":"9"})).unwrap(),json!({"target":"mask","contents":"background"}));
        }
    }
    #[test]
    fn partial_noncanonical_and_overflow_pairs_are_not_silently_weakened() {
        for params in [json!({"documentId":"1"}),json!({"documentId":"01","expectedRevision":"0"}),json!({"documentId":"0","expectedRevision":"0"}),json!({"documentId":"1","expectedRevision":0}),json!({"documentId":"1","expectedRevision":"18446744073709551616"})] {
            assert!(normalize("layer.layerMask.hideSelection",params).is_err());
        }
    }
    #[test]
    fn flush_keeps_original_journal_and_does_not_advance_on_malformed_pair() {
        let mut session=crate::Session::new();
        session.execute("actions.record",json!({"name":"Own relative mask"})).unwrap();
        let params=json!({"layer":77,"documentId":"123","expectedRevision":"8"});
        session.journal.push(("layer.layerMask.hideAll".into(),params.clone()));
        super::super::flush_recording(&mut session).unwrap();
        assert_eq!(session.journal.last().unwrap().1,params);
        assert_eq!(session.actions.list[0].steps[0].1,json!({"layer":77}));
        session.journal.push(("edit.copy".into(),json!({"documentId":"1"})));
        let before=session.actions.clone();
        assert!(super::super::flush_recording(&mut session).is_err());
        assert_eq!(session.actions.list,before.list);
        assert_eq!(session.actions.recording,before.recording);
        assert_eq!(session.actions.rev,before.rev);
    }
}
