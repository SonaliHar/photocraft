//! Deferred producer equivalence and control contracts. Source Surfaces and
//! expected arrays are external harness state, not actual owned document claims.
use super::*;
use photocraft_raster::backing::quota::{QuotaLimits,QuotaUsage};
fn ctl()->ReadControl<'static>{ReadControl{interrupt:&photocraft_raster::Interrupt::NONE,work:&|_|Ok(())}}
fn quota()->BackingQuota{BackingQuota::new_with_aggregate_initial(QuotaLimits{resident:512<<20,index:64<<20,scratch:0},512<<20,QuotaUsage::default()).unwrap().0}
#[test]
#[ignore="UNRUN/uncompiled native borrowed Round equals existing resident producer on RGB/Gray U8/U16/F32, negative-coordinate clipping, erase/pencil settings and locked alpha; internal equivalence only."]
fn clipped_native_matches_existing_resident_words(){
 for mode in [photocraft_color::ColorMode::Rgb,photocraft_color::ColorMode::Grayscale]{for sample in [SampleType::U8,SampleType::U16,SampleType::F32]{for erase in [false,true]{for lock in [false,true]{for alpha in [0.0,0.75,1.0]{for aliased in [false,true]{
  let format=PixelFormat::new(mode,sample,true);let mut default=vec![0.25;format.channels()];*default.last_mut().unwrap()=alpha;let source=Surface::with_default(format,&default);let mut expected=source.clone();let bounds=Rect::new(-3,-2,258,260);
  let brush=BrushSettings{aliased,size:7.0,hardness:0.6,pressure_size:false,opacity:0.7,flow:0.8,erase,seed:53,color:[0.8,0.2,0.1,0.75],..Default::default()};let points=[StrokePoint::new(-1.25,-0.5,0.7),StrokePoint::new(3.0,4.0,1.0)];
  crate::render_stroke_with_interrupt(&mut expected,&brush,&points,None,lock,1.0,crate::StrokeLimits::default(),&photocraft_raster::Interrupt::NONE).unwrap();
  let q=quota();let baseline=q.usage().unwrap();let view=CheckedRasterView::resident(&source,bounds).unwrap();let plan=prepare_clipped_native_stroke(view,bounds,&brush,&points,None,lock,1.0,NativeStrokeLimits::STANDARD,&q,&ctl()).unwrap();let patch=plan.render(&ctl()).unwrap();let rect=patch.bounds();let bpp=format.bytes_per_pixel();let mut raw=vec![0;bpp];
  for y in rect.y0..rect.y1{for x in rect.x0..rect.x1{expected.try_read_pixel_bytes(x,y,&mut raw).unwrap();let at=((y-rect.y0)as usize*rect.width()as usize+(x-rect.x0)as usize)*bpp;assert_eq!(&patch.bytes()[at..at+bpp],raw.as_slice());}}
  drop(patch);assert_eq!(q.usage().unwrap(),baseline);
 }}}}}}
}
#[test]
#[ignore="UNRUN/uncompiled prospective quota/control refusal, original forwarded Work/Cancelled, unsupported settings and nonfinite arithmetic alpha atomicity. No completion/performance claim."]
fn native_controls_refund_and_reject_before_publication(){
 let q=quota();let source=Surface::new(PixelFormat::RGBA8);let bounds=Rect::new(0,0,257,259);let brush=BrushSettings{size:5.0,pressure_size:false,..Default::default()};let points=[StrokePoint::new(255.0,256.0,1.0)];let before=source.clone();let usage=q.usage().unwrap();
 let work=|_|Err(BackingError::Work);let deny=ReadControl{interrupt:&photocraft_raster::Interrupt::NONE,work:&work};assert!(matches!(prepare_clipped_native_stroke(CheckedRasterView::resident(&source,bounds).unwrap(),bounds,&brush,&points,None,false,1.0,NativeStrokeLimits::STANDARD,&q,&deny),Err(NativeStrokeError::Backing(BackingError::Work))));
 let yes=||true;let interrupt=photocraft_raster::Interrupt::cancel_only(&yes);let c=ReadControl{interrupt:&interrupt,work:&|_|Ok(())};assert!(prepare_clipped_native_stroke(CheckedRasterView::resident(&source,bounds).unwrap(),bounds,&brush,&points,None,false,1.0,NativeStrokeLimits::STANDARD,&q,&c).is_err());
 assert!(prepare_clipped_native_stroke(CheckedRasterView::resident(&source,bounds).unwrap(),bounds,&brush,&points,None,false,1.0,NativeStrokeLimits::try_lowered(1,8_000_000_000,16<<20).unwrap(),&q,&ctl()).is_err());
 let mut bad=brush.clone();bad.square=true;assert!(checked_round_brush(&bad).is_err());bad=brush;bad.color[3]=f32::NAN;assert!(prepare_clipped_native_stroke(CheckedRasterView::resident(&source,bounds).unwrap(),bounds,&bad,&points,None,false,1.0,NativeStrokeLimits::STANDARD,&q,&ctl()).is_err());assert_eq!(source,before);assert_eq!(q.usage().unwrap(),usage);
}
