//! Opt-in exact retained configuration -> owned effective CMS -> existing graph
//! callback. No import qualification, outer filter policy or port name is guessed.
use super::*;
use crate::gimp_configured_candidate::RetainedSchemaIdentity;
use crate::gimp_configured_color_candidate::{OwnedConfiguredColorCandidate,prepare_profile_owned_color_candidate};
use photocraft_algo::gimp::configured::WorkingContext;
use photocraft_cms::{Profile,xcf_color::Encoding};
use photocraft_raster::Interrupt;

pub struct ProfileOwnedParameters<'a> {
 pub node:GraphNodeId,
 pub version_origin:GraphVersionOrigin,
 pub schema:RetainedSchemaIdentity<'a>,
 pub source_domain:ExecutionDomain,
 pub source_extent:Rect,
 pub working_default:&'a [u8],
 pub input_port:&'a str,
 pub upstream_output_port:&'a str,
 pub output_port:&'a str,
 pub output_policy:OutputPolicy,
}
pub struct ProfileOwnedBinding<'a> {
 parameters:ProfileOwnedParameters<'a>,
 candidate:OwnedConfiguredColorCandidate<'a>,
}
/// Actual constructor-owned storage/work, not a caller-asserted source census.
#[derive(Clone,Copy,Debug)]
pub struct ProfileOwnedBindingReceipt {
 pub retained_bytes:u64,
 pub preparation_work_units:u64,
}
impl<'a> ProfileOwnedBinding<'a> {
 pub fn parameters(&self)->&ProfileOwnedParameters<'a>{&self.parameters}
 pub fn original_node(&self)->&FilterNode{self.candidate.original_node()}
 pub fn requirements_receipt(&self)->ProfileOwnedBindingReceipt{ProfileOwnedBindingReceipt{retained_bytes:self.candidate.retained_bytes(),preparation_work_units:self.candidate.preparation_work()}}
 pub(crate) fn execution_parameters(&self)->&ProfileOwnedParameters<'a>{&self.parameters}
 pub(crate) fn execution_original(&self)->&FilterNode{self.candidate.original_node()}
 pub(crate) fn execution_retained(&self)->u64{self.candidate.retained_bytes()}
 pub(crate) fn execution_preparation_work(&self)->u64{self.candidate.preparation_work()}
 /// Execute an explicitly bound graph with already-owned sources. Original
 /// source schema, outer composition and filesystem authority are not inferred.
 #[allow(clippy::too_many_arguments)]
 pub fn execute_graph<'s>(graph:&photocraft_doc::xcf_filters::FilterGraph,bindings:&[&ProfileOwnedBinding<'_>],sources:&[crate::gimp_configured_graph_execution::OwnedSource<'_>],backing:&[&[u8]],external:Option<&'s photocraft_raster::Surface>,roi:Rect,external_domain:ExecutionDomain,assignments:&[photocraft_algo::gimp::graph_execute::DomainAssignment],max_depth:usize,limits:photocraft_algo::gimp::graph_execute::ExecutionLimits,control:&Interrupt<'_>)->Result<photocraft_algo::gimp::graph_execute::GraphOutput<'s>,photocraft_algo::gimp::graph_pipeline::GraphPipelineError>{
  crate::gimp_configured_graph_execution::execute(graph,bindings,sources,backing,external,roi,external_domain,assignments,max_depth,limits,control)
 }
}
fn valid(p:&ProfileOwnedParameters<'_>)->bool {
 p.node.0!=0&&!p.source_extent.is_empty()&&[p.input_port,p.upstream_output_port,p.output_port].iter().all(|name|!name.is_empty()&&name.len()<=256&&!name.contains('\0'))
}
/// Called before scheduler/source ROI allocation. Actual stored selector/TRC
/// configuration and explicit current profile basis are used by the owned factory.
/// Caller supplies the exact graph node/version/ports and native domain; this is
/// candidate preparation, not inferred GEGL chain semantics or import admission.
#[allow(clippy::too_many_arguments)]
pub fn prepare_profile_owned_binding<'a>(node:&'a FilterNode,parameters:ProfileOwnedParameters<'a>,basis:&Profile,current_encoding:Encoding,actual_source:WorkingContext,limits:Limits,control:&Interrupt<'_>)->Result<ProfileOwnedBinding<'a>,RasterInputError>{
 control.check().map_err(|_|RasterInputError::Cancelled)?;
 if !valid(&parameters)||!matches!(parameters.source_domain.format.mode,photocraft_color::ColorMode::Rgb|photocraft_color::ColorMode::Grayscale)||parameters.source_domain.profile_fingerprint!=Some(actual_source.profile_identity)||parameters.source_domain.encoding!=encoding(actual_source.transfer)||parameters.source_domain.format.mode.color_channels()!=actual_source.color_channels||parameters.source_domain.format.alpha!=actual_source.alpha{return Err(RasterInputError::UnsupportedConversion);}
 let format=photocraft_color::PixelFormat::new(if actual_source.color_channels==3{photocraft_color::ColorMode::Rgb}else{photocraft_color::ColorMode::Grayscale},photocraft_color::SampleType::F64,actual_source.alpha);
 if parameters.working_default.len()!=format.bytes_per_pixel(){return Err(RasterInputError::LengthMismatch);}
 // Validate the caller's actual default now, before any later native region read.
 for index in 0..format.channels(){control.check().map_err(|_|RasterInputError::Cancelled)?;let value=photocraft_color::try_read_sample_f64(parameters.working_default,photocraft_color::SampleType::F64,index).map_err(|_|RasterInputError::InvalidSample)?;if !value.is_finite()||(actual_source.alpha&&index==actual_source.color_channels&&!(0.0..=1.0).contains(&value)){return Err(RasterInputError::InvalidSample);}}
 let candidate=prepare_profile_owned_color_candidate(node,parameters.schema,basis,current_encoding,actual_source,limits,control)?;
 Ok(ProfileOwnedBinding{parameters,candidate})
}
pub fn accepts_profile_owned_port(binding:&ProfileOwnedBinding<'_>,direction:PortDirection,port:&str)->bool {
 valid(&binding.parameters)&&match direction{PortDirection::Input=>port==binding.parameters.input_port,PortDirection::Output=>port==binding.parameters.output_port}
}
pub fn footprint_profile_owned(binding:&ProfileOwnedBinding<'_>,request:FootprintRequest<'_>)->Result<InputFootprint,GraphRoiError>{
 let p=&binding.parameters;
 if !valid(p)||request.node!=p.node||request.edge.to!=p.node||request.edge.input_port!=p.input_port||request.edge.output_port!=p.upstream_output_port||request.output.is_empty()||request.output.intersect(&p.source_extent)!=request.output{return Err(GraphRoiError::UnsupportedFootprint);}
 let work=binding.candidate.pair().work_per_pixel.checked_add(16384).ok_or(GraphRoiError::WorkLimit)?;
 Ok(InputFootprint{mapped_output:request.output,source_extent:p.source_extent,halo:Halo::default(),abyss:Abyss::None,work_per_sample:work})
}
/// Reuses the existing prepared color callback, with exact current graph arguments
/// bound to the original retained source arguments. No external validator/size
/// assertion can replace that binding. Every cell conservatively prepays owned
/// constructor work and config scratch; upstream reservations are not refunded.
pub fn evaluate_profile_owned(binding:&ProfileOwnedBinding<'_>,request:NodeRequest<'_>,budget:&mut NodeBudget<'_, '_>)->Result<NodeOutput,GraphExecuteError>{
 budget.poll()?;let p=&binding.parameters;let original=binding.candidate.original_node();
 if !valid(p)||request.node.id!=p.node||request.node.operation!=original.operation||request.node.version_origin!=p.version_origin||request.domain!=p.source_domain||request.inputs.len()!=1{return Err(GraphExecuteError::Domain);}
 budget.charge_work(binding.candidate.preparation_work())?;
 budget.charge_work(16384)?;budget.reserve_scratch(65536)?;
 // Comparing against the admitted retained shape cannot descend into an
 // arbitrary same-shape graph: the original contains only five bounded scalar
 // selector groups. Reject changed arguments before preparing owned config.
 if request.node.arguments!=original.arguments{return Err(GraphExecuteError::Operation("retained configured arguments changed"));}
 let control=*budget.interrupt();let pair=binding.candidate.pair();let mut outcome=None;
 binding.candidate.with_prepared(&control,|prepared|{
  let validate=|node:&photocraft_doc::xcf_filters::GraphNode,_:&photocraft_algo::gimp::configured::PreparedConfiguration<'_>|{
   if node.arguments!=original.arguments{return Err(GraphExecuteError::Operation("retained configured arguments changed"));}Ok(())
  };
  let borrowed=PreparedBinding{node:p.node,identity:&original.operation,version_origin:p.version_origin,prepared,source_domain:p.source_domain,source_extent:p.source_extent,transforms:&pair,working_default:p.working_default,input_port:p.input_port,upstream_output_port:p.upstream_output_port,output_port:p.output_port,output_policy:p.output_policy,validate_arguments:&validate,validation_bytes:0,validation_work:16384};
  outcome=Some(evaluate_prepared(&borrowed,request,budget));Ok(())
 }).map_err(raster)?;
 outcome.ok_or(GraphExecuteError::Domain)?
}
pub fn evaluate_profile_owned_available(binding:&ProfileOwnedBinding<'_>,request:AvailableNodeRequest<'_>,budget:&mut NodeBudget<'_, '_>)->Result<NodeOutput,GraphExecuteError>{
 graph_execute::evaluate_materialized(request,budget,|request,budget|evaluate_profile_owned(binding,request,budget))
}

#[cfg(test)]
mod tests {
 use super::*;
 use crate::gimp_configured_candidate::LEVELS_SCHEMA;
 use crate::gimp_configured_render_candidate::CoveragePolicy;
 use photocraft_algo::gimp::{configured::Transfer,graph_execute::{DomainAssignment,ExecutionLimits},graph_pipeline::execute_pipeline_with_roi_domains};
 use photocraft_color::{ColorMode,PixelFormat,SampleType,checked_arithmetic::{Policy,Domain,Rounding,UnitOverflow}};
 use photocraft_doc::{item_identity::FilterId,xcf_filters::{FilterArgument,FilterValue,ExactNumber,OperationIdentity,GraphNode,GraphEdge,FilterGraph}};
 use photocraft_raster::Surface;
 use sha2::{Digest,Sha256};

 fn fixture()->FilterNode {
  let channels=["value","red","green","blue","alpha"].map(|channel|FilterArgument{name:channel.into(),value:FilterValue::Config([("low-input",0.0f64),("high-input",1.0),("gamma",1.0),("low-output",if channel=="alpha"{0.25}else{0.0}),("high-output",1.0)].map(|(name,value)|FilterArgument{name:name.into(),value:FilterValue::Number(ExactNumber::Float64Bits(value.to_bits()))}).into())});
  FilterNode{id:FilterId(1),operation:OperationIdentity{name:"gimp:levels".into(),stored_version:"0:0".into(),schema_epoch:"owned-graph-test".into()},source_display_name:None,source_icon:None,enabled:true,opacity_f64_bits:1f64.to_bits(),source_clipping:Some(0),source_region:Some(0),mask:None,auxiliary:vec![],composition:None,arguments:vec![FilterArgument{name:"trc".into(),value:FilterValue::Enum{domain:"gimp:levels::trc".into(),value:1}},FilterArgument{name:"clamp-input".into(),value:FilterValue::Bool(false)},FilterArgument{name:"clamp-output".into(),value:FilterValue::Bool(false)},FilterArgument{name:"config".into(),value:FilterValue::Config(channels.into())}]}
 }
 fn context(profile:&Profile)->WorkingContext {WorkingContext{transfer:Transfer::NonlinearProfile,profile_identity:Sha256::digest(profile.to_bytes().as_slice()).into(),color_channels:3,alpha:true}}
 fn limits()->Limits {Limits{resident_bytes:256<<20,work_units:1_000_000_000}}
 fn parameters<'a>(actual:WorkingContext,default:&'a [u8])->ProfileOwnedParameters<'a>{
  let policy=Policy{domain:Domain::Unit,unit_overflow:UnitOverflow::Reject,rounding:Rounding::Nearest};
  ProfileOwnedParameters{node:GraphNodeId(2),version_origin:GraphVersionOrigin::Stored,schema:RetainedSchemaIdentity{xcf_version:22,expected_epoch:"owned-graph-test",schema_sha256:LEVELS_SCHEMA},source_domain:ExecutionDomain{format:PixelFormat::new(ColorMode::Rgb,SampleType::F32,true),encoding:SampleEncoding::NonlinearProfile,profile_fingerprint:Some(actual.profile_identity)},source_extent:Rect::new(-1,0,1,1),working_default:default,input_port:"input",upstream_output_port:"output",output_port:"output",output_policy:OutputPolicy{colors:policy,alpha:policy,coverage:CoveragePolicy::EvaluatedRejectUnit}}
 }
 fn graph(node:&FilterNode)->FilterGraph{FilterGraph{input:Some(GraphNodeId(1)),nodes:vec![GraphNode{id:GraphNodeId(2),operation:node.operation.clone(),version_origin:GraphVersionOrigin::Stored,arguments:node.arguments.clone()}],edges:vec![GraphEdge{from:GraphNodeId(1),to:GraphNodeId(2),input_port:"input".into(),output_port:"output".into()}],output:GraphNodeId(2),original_pipeline:None}}
 #[test]
 fn graph_callback_consumes_stored_alpha_selector_and_matches_owned_render_without_source_mutation(){
  let node=fixture();let original=node.clone();let basis=photocraft_cms::Builtin::Srgb.profile();let encoded=basis.to_bytes();let actual=context(basis);let default=[0u8;32];
  let binding=prepare_profile_owned_binding(&node,parameters(actual,&default),basis,Encoding::NonlinearProfile,actual,limits(),&Interrupt::NONE).unwrap();
  let domain=binding.parameters.source_domain;let roi=binding.parameters.source_extent;let mut source=Surface::new(domain.format);source.write_pixel(-1,0,&[0.2,0.4,0.6,0.0]);source.write_pixel(0,0,&[0.1,0.3,0.5,1.0]);let before=source.read_region(roi);
  let direct=binding.candidate.render(&source,roi,roi,actual,&default,binding.parameters.output_policy,limits(),&Interrupt::NONE).unwrap();
  let graph=graph(&node);let assignments=[DomainAssignment{node:GraphNodeId(2),domain}];
  let output=execute_pipeline_with_roi_domains(&graph,2,Some(&source),roi,domain,&assignments,ExecutionLimits::default(),&Interrupt::NONE,|_,direction,port|accepts_profile_owned_port(&binding,direction,port),|request|footprint_profile_owned(&binding,request),|request,budget|evaluate_profile_owned(&binding,request,budget)).unwrap();
  assert_eq!(output.surface().read_region(roi),direct.surface.read_region(roi));assert_eq!(output.surface().read_region(roi)[3],0.25);assert_eq!(source.read_region(roi),before);assert_eq!(node,original);assert_eq!(basis.to_bytes().as_slice(),encoded.as_slice());
 }
 #[test]
 fn changed_graph_config_rejects_and_invalid_default_or_cancellation_never_prepare_source(){
  let node=fixture();let basis=photocraft_cms::Builtin::Srgb.profile();let actual=context(basis);let default=[0u8;32];let binding=prepare_profile_owned_binding(&node,parameters(actual,&default),basis,Encoding::NonlinearProfile,actual,limits(),&Interrupt::NONE).unwrap();
  let mut graph=graph(&node);graph.nodes[0].arguments[1].value=FilterValue::Bool(true);let domain=binding.parameters.source_domain;let source=Surface::new(domain.format);let assignments=[DomainAssignment{node:GraphNodeId(2),domain}];
  assert!(execute_pipeline_with_roi_domains(&graph,2,Some(&source),binding.parameters.source_extent,domain,&assignments,ExecutionLimits::default(),&Interrupt::NONE,|_,direction,port|accepts_profile_owned_port(&binding,direction,port),|request|footprint_profile_owned(&binding,request),|request,budget|evaluate_profile_owned(&binding,request,budget)).is_err());
  assert!(matches!(prepare_profile_owned_binding(&node,parameters(actual,&[0u8;8]),basis,Encoding::NonlinearProfile,actual,limits(),&Interrupt::NONE),Err(RasterInputError::LengthMismatch)));
  let stop=||true;assert!(matches!(prepare_profile_owned_binding(&node,parameters(actual,&default),basis,Encoding::NonlinearProfile,actual,limits(),&Interrupt::cancel_only(&stop)),Err(RasterInputError::Cancelled)));
  assert_eq!(node,fixture());
 }
}
