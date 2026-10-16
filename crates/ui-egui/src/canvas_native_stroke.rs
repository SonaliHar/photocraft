//! Press-time captured native one-shot lane; no resident live-preview fallback.
use super::*;
#[derive(Clone,Debug)]
pub(super) struct Target{
 engine:std::sync::Arc<photocraft_engine::AccountedNativeStrokeTarget>,
 command:&'static str,tool:Tool,zoom:u32,auto_erase:bool,block:bool,pencil:bool,
}
pub(super) fn capture(app:&PhotocraftApp,tool:Tool)->Result<Option<Target>,String>{
 if !matches!(tool,Tool::Brush|Tool::Pencil|Tool::Eraser)||!app.session.active().is_some_and(|s|s.doc.accounted().is_some()){return Ok(None)}
 let engine=photocraft_engine::capture_accounted_native_stroke_target(&app.session).map_err(|e|e.to_string())?.ok_or_else(||"native brush painting is unavailable for this readonly storage class".to_owned())?;
 engine.admit_samples(2).map_err(|e|e.to_string())?;
 Ok(Some(Target{engine,tool:app.ui.tool,command:stroke_command(app,tool),zoom:app.point_zoom().to_bits(),auto_erase:app.ui.tool_options.pencil_auto_erase,block:crate::eraser_ui::block_mode(app,tool),pencil:crate::eraser_ui::pencil_mode(app,tool)}))
}
pub(super) fn commit(app:&mut PhotocraftApp,d:&Drag)->bool{
 let Some(target)=&d.native_stroke_target else{return false};
 // A captured source can never fall through to a different resident target.
 app.live_stroke=None;app.session.clear_live_stroke();
 if app.ui.tool!=target.tool||app.point_zoom().to_bits()!=target.zoom||stroke_command(app,d.tool)!=target.command||app.ui.tool_options.pencil_auto_erase!=target.auto_erase||crate::eraser_ui::block_mode(app,d.tool)!=target.block||crate::eraser_ui::pencil_mode(app,d.tool)!=target.pencil{
  app.ui.status="The pressed native brush zoom or tool options changed before release".into();app.ui.status_error=true;return true;
 }
 if let Err(error)=target.engine.admit_samples(d.points.len()){app.ui.status=error.to_string();app.ui.status_error=true;return true;}
 if let Err(error)=target.engine.visit_samples(d.points.len()){app.ui.status=error.to_string();app.ui.status_error=true;return true;}
 let mut params=stroke_params(app,d,&app.stylus.stroke_points(&d.points));
 if let Err(error)=target.engine.inject_captured_params(&app.session,&mut params){app.ui.status=error.to_string();app.ui.status_error=true;return true;}
 if let Err(error)=target.engine.arm_captured_work(&mut app.session){app.ui.status=error.to_string();app.ui.status_error=true;return true;}
 let _=app.run(target.command,params);target.engine.clear_captured_work(&mut app.session);true
}

pub(super) fn admit(d:&Drag,count:usize)->Result<(),String>{if let Some(target)=&d.native_stroke_target{target.engine.admit_samples(count).map_err(|e|e.to_string())?;}Ok(())}

pub(super) fn visit(d:&Drag)->Result<(),String>{if let Some(target)=&d.native_stroke_target{target.engine.visit_samples(d.points.len()).map_err(|e|e.to_string())?;}Ok(())}
