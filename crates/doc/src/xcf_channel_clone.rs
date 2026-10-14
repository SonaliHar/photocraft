//! Selective channel-metadata staging. Global source/resource/selection state is
//! borrowed and validated, never cloned merely to obtain atomic publication.
#[cfg(test)]
#[path = "stored_color_clone_tests.rs"]
mod stored_color_clone_tests;
use super::{invalid, Result};
use crate::{Document, XcfState, FilterMask, item_identity::*, xcf_state::ItemState, xcf_filters::*, xcf_items::EditableSource, xcf_provenance::OpaqueRecord, import_compositing::GimpComposition};
use photocraft_raster::{Interrupt, Surface};
use std::{collections::{BTreeMap,HashSet},mem::size_of,sync::Arc};
fn poll(control:&Interrupt<'_>)->Result<()> {control.check().map_err(|_|invalid("cancelled"))}
fn add(value:&mut u64,amount:u64)->Result<()> {*value=value.checked_add(amount).ok_or_else(||invalid("clone_channel_limit"))?;Ok(())}
pub(super) fn reserved<T>(count:usize)->Result<Vec<T>> {
    let bytes=count.checked_mul(size_of::<T>()).filter(|n|*n<=256<<20).ok_or_else(||invalid("clone_channel_limit"))?;
    let mut output=Vec::new();output.try_reserve_exact(count).map_err(|_|invalid("clone_channel_limit"))?;
    if output.capacity().checked_mul(size_of::<T>()).is_none_or(|n|n>bytes){return Err(invalid("clone_channel_limit"));}Ok(output)
}
pub(super) fn string(value:&str,control:&Interrupt<'_>)->Result<String> {
    if value.len()>1<<20{return Err(invalid("string_limit"));}
    let mut bytes=reserved(value.len())?;
    for chunk in value.as_bytes().chunks(32<<10){poll(control)?;if chunk.contains(&0){return Err(invalid("string_limit"));}bytes.extend_from_slice(chunk);}
    String::from_utf8(bytes).map_err(|_|invalid("string_limit"))
}
pub(super) fn optional(value:Option<&str>,control:&Interrupt<'_>)->Result<Option<String>> {value.map(|value|string(value,control)).transpose()}
pub(super) fn copied<T:Clone>(value:&[T],control:&Interrupt<'_>)->Result<Vec<T>> {
    let mut output=reserved(value.len())?;
    for chunk in value.chunks(256){poll(control)?;output.extend_from_slice(chunk);}Ok(output)
}
fn composition(value:Option<&GimpComposition>,control:&Interrupt<'_>)->Result<Option<GimpComposition>> {
    value.map(|value|Ok(GimpComposition{compatibility_epoch:string(&value.compatibility_epoch,control)?,source_mode:value.source_mode,legacy:value.legacy,stored_blend_space:value.stored_blend_space,stored_composite_space:value.stored_composite_space,stored_operator:value.stored_operator,resolved:value.resolved.clone(),pass_through:value.pass_through})).transpose()
}
fn identity(value:&OperationIdentity,control:&Interrupt<'_>)->Result<OperationIdentity> {Ok(OperationIdentity{name:string(&value.name,control)?,stored_version:string(&value.stored_version,control)?,schema_epoch:string(&value.schema_epoch,control)?})}
pub(super) fn arguments(values:&[FilterArgument],depth:usize,control:&Interrupt<'_>)->Result<Vec<FilterArgument>> {
    if depth>32{return Err(invalid("config_limit"));}
    let mut output=reserved(values.len())?;
    for argument in values {
        poll(control)?;
        let value=match &argument.value {
            FilterValue::Number(number)=>FilterValue::Number(number.clone()),FilterValue::Bool(value)=>FilterValue::Bool(*value),FilterValue::Resource(id)=>FilterValue::Resource(*id),
            FilterValue::String(value)=>FilterValue::String(string(value,control)?),
            FilterValue::Enum{domain,value}=>FilterValue::Enum{domain:string(domain,control)?,value:*value},
            FilterValue::Color{components,encoding,profile}=>FilterValue::Color{components:copied(components,control)?,encoding:*encoding,profile:*profile},
            FilterValue::StoredColor(ExactColorStorage{layout,samples,opaque_tail})=>{
                if !stored_color_shape(layout,samples,opaque_tail){return Err(invalid("stored_color_storage"));}
                FilterValue::StoredColor(ExactColorStorage{layout:string(layout,control)?,samples:copied(samples,control)?,opaque_tail:copied(opaque_tail,control)?})
            },
            FilterValue::Curve{free,points_f64_bits,point_types,samples_f64_bits}=>FilterValue::Curve{free:*free,points_f64_bits:copied(points_f64_bits,control)?,point_types:copied(point_types,control)?,samples_f64_bits:copied(samples_f64_bits,control)?},
            FilterValue::Config(config)=>FilterValue::Config(arguments(config,depth+1,control)?),
            FilterValue::Graph(graph)=>{
                let mut nodes=reserved(graph.nodes.len())?;
                for node in &graph.nodes{poll(control)?;nodes.push(GraphNode{id:node.id,operation:identity(&node.operation,control)?,version_origin:node.version_origin,arguments:arguments(&node.arguments,depth+2,control)?});}
                let mut edges=reserved(graph.edges.len())?;
                for edge in &graph.edges{poll(control)?;edges.push(GraphEdge{from:edge.from,to:edge.to,input_port:string(&edge.input_port,control)?,output_port:string(&edge.output_port,control)?});}
                FilterValue::Graph(FilterGraph{input:graph.input,nodes,edges,output:graph.output,original_pipeline:optional(graph.original_pipeline.as_deref(),control)?})
            }
        };
        output.push(FilterArgument{name:string(&argument.name,control)?,value});
    }
    Ok(output)
}
pub(super) fn item(value:&ItemState,control:&Interrupt<'_>)->Result<ItemState> {Ok(ItemState{item:value.item,name:string(&value.name,control)?,custom_name:value.custom_name,visible:value.visible,locks:value.locks,parent:value.parent,source_tattoo:value.source_tattoo,source_flags:value.source_flags,raster_bounds:value.raster_bounds,mask:value.mask.clone(),composition:composition(value.composition.as_ref(),control)?})}
pub(super) fn node(value:&FilterNode,control:&Interrupt<'_>)->Result<FilterNode> {
    let mut auxiliary=reserved(value.auxiliary.len())?;
    for (port,input) in &value.auxiliary{poll(control)?;auxiliary.push((string(port,control)?,*input));}
    Ok(FilterNode{id:value.id,operation:identity(&value.operation,control)?,source_display_name:optional(value.source_display_name.as_deref(),control)?,source_icon:optional(value.source_icon.as_deref(),control)?,arguments:arguments(&value.arguments,0,control)?,enabled:value.enabled,opacity_f64_bits:value.opacity_f64_bits,source_clipping:value.source_clipping,source_region:value.source_region,mask:value.mask,auxiliary,composition:composition(value.composition.as_ref(),control)?})
}
pub(super) fn surface(source:&Surface,control:&Interrupt<'_>)->Result<Surface> {
    source.try_validate_tile_origins_with(control).map_err(|_|invalid(if control.cancelled(){"cancelled"}else{"clone_channel_mask"}))?;
    let mut output=Surface::try_with_default_bytes(source.format(),source.default_bytes()).map_err(|_|invalid("clone_channel_limit"))?;
    // std BTreeMap nodes have no stable fallible allocator API. Each bounded
    // publication is prepaid and polled; raw tile Arcs are never copied.
    for (coordinate,tile) in source.tiles(){poll(control)?;output.put_tiles(std::iter::once((*coordinate,Arc::clone(tile))));}
    poll(control)?;Ok(output)
}
pub(super) struct Census<'a,'b> {pub(super) control:&'a Interrupt<'b>,pub(super) model:u64,pub(super) payload:u64,pub(super) work:u64,pub(super) visits:usize,pub(super) seen:HashSet<(u8,usize)>}
impl Census<'_,'_> {
    pub(super) fn bytes(&mut self,value:u64)->Result<()> {poll(self.control)?;add(&mut self.model,value)?;if self.model>256<<20{return Err(invalid("clone_channel_limit"));}Ok(())}
    pub(super) fn text(&mut self,value:&String)->Result<()> {self.bytes(value.capacity() as u64)?;add(&mut self.work,value.len() as u64)}
    pub(super) fn array<T>(&mut self,value:&Vec<T>)->Result<()> {self.bytes((value.capacity() as u64).checked_mul(size_of::<T>() as u64).ok_or_else(||invalid("clone_channel_limit"))?)}
    pub(super) fn arc(&mut self,value:&Arc<Vec<u8>>)->Result<()> {poll(self.control)?;add(&mut self.work,value.len() as u64)?;let key=(0,Arc::as_ptr(value) as usize);if !self.seen.contains(&key){if self.seen.len()>=self.seen.capacity(){return Err(invalid("clone_channel_limit"));}self.seen.insert(key);add(&mut self.payload,(value.capacity() as u64).checked_add((size_of::<Vec<u8>>()+2*size_of::<usize>()) as u64).ok_or_else(||invalid("clone_channel_limit"))?)?;}Ok(())}
    pub(super) fn surface(&mut self,value:&Surface)->Result<()> {
        self.bytes(value.default_bytes().len() as u64)?;self.bytes((value.tile_count() as u64).checked_mul(256).ok_or_else(||invalid("clone_channel_limit"))?)?;
        for (_,tile) in value.tiles(){poll(self.control)?;add(&mut self.work,tile.bytes().len() as u64)?;let key=(1,Arc::as_ptr(tile) as usize);if !self.seen.contains(&key){if self.seen.len()>=self.seen.capacity(){return Err(invalid("clone_channel_limit"));}self.seen.insert(key);add(&mut self.payload,(tile.bytes().len() as u64).checked_add((size_of::<photocraft_raster::Tile>()+2*size_of::<usize>()) as u64).ok_or_else(||invalid("clone_channel_limit"))?)?;}}
        Ok(())
    }
    pub(super) fn composition(&mut self,value:Option<&GimpComposition>)->Result<()> {if let Some(value)=value{self.text(&value.compatibility_epoch)?;}Ok(())}
    pub(super) fn arguments(&mut self,args:&Vec<FilterArgument>,depth:usize)->Result<()> {
        if depth>32{return Err(invalid("config_limit"));}self.array(args)?;
        for argument in args {
            poll(self.control)?;self.visits=self.visits.checked_add(1).filter(|n|*n<=100_000).ok_or_else(||invalid("config_limit"))?;self.text(&argument.name)?;
            match &argument.value {
                FilterValue::String(value)=>self.text(value)?,FilterValue::Enum{domain,..}=>self.text(domain)?,
                FilterValue::Color{components,..}=>{self.array(components)?;add(&mut self.work,components.len() as u64)?;},
                FilterValue::StoredColor(ExactColorStorage{layout,samples,opaque_tail})=>{
                    if !stored_color_shape(layout,samples,opaque_tail){return Err(invalid("stored_color_storage"));}
                    self.text(layout)?;self.array(samples)?;self.array(opaque_tail)?;
                    for chunk in samples.chunks(32<<10).chain(opaque_tail.chunks(32<<10)){poll(self.control)?;add(&mut self.work,chunk.len() as u64)?;}
                },
                FilterValue::Curve{points_f64_bits,point_types,samples_f64_bits,..}=>{self.array(points_f64_bits)?;self.array(point_types)?;self.array(samples_f64_bits)?;add(&mut self.work,(points_f64_bits.len()+point_types.len()+samples_f64_bits.len()) as u64)?;},
                FilterValue::Config(config)=>self.arguments(config,depth+1)?,
                FilterValue::Graph(graph)=>{
                    graph.charge_validation_work(depth+1,&mut self.visits,self.control)?;self.array(&graph.nodes)?;self.array(&graph.edges)?;
                    for node in &graph.nodes{poll(self.control)?;self.identity(&node.operation)?;self.arguments(&node.arguments,depth+2)?;}
                    for edge in &graph.edges{poll(self.control)?;self.text(&edge.input_port)?;self.text(&edge.output_port)?;}
                    if let Some(value)=&graph.original_pipeline{self.text(value)?;}
                }
                _=>{}
            }
        }
        Ok(())
    }
    pub(super) fn identity(&mut self,value:&OperationIdentity)->Result<()> {self.text(&value.name)?;self.text(&value.stored_version)?;self.text(&value.schema_epoch)}
    pub(super) fn item(&mut self,value:&ItemState)->Result<()> {self.text(&value.name)?;self.composition(value.composition.as_ref())}
    pub(super) fn stack(&mut self,value:&FilterStack)->Result<()> {
        self.array(&value.nodes)?;
        for node in &value.nodes{poll(self.control)?;self.identity(&node.operation)?;if let Some(value)=&node.source_display_name{self.text(value)?;}if let Some(value)=&node.source_icon{self.text(value)?;}self.arguments(&node.arguments,0)?;self.array(&node.auxiliary)?;for (port,_) in &node.auxiliary{self.text(port)?;}self.composition(node.composition.as_ref())?;}
        Ok(())
    }
    pub(super) fn opaque(&mut self,value:&OpaqueRecord)->Result<()> {self.text(&value.namespace)?;self.text(&value.identity)?;self.text(&value.blake3)?;self.arc(&value.bytes)}
}

pub(super) fn clone_channel(state:&mut XcfState,source:ChannelId,target:ChannelId,doc:&Document,filter_ids:&BTreeMap<FilterId,FilterId>,control:&Interrupt<'_>)->Result<()> {
    poll(control)?;
    if source==target || source.0==0 || target.0==0 || doc.channels.len()>10_000 || doc.paths.len()>10_000 || filter_ids.len()>100_000{return Err(invalid("clone_channel"));}
    if state.items.len()>10_000 || state.resources.len()>10_000 || state.source_items.len()>10_000 || state.editable_indexed.len()>10_000 || state.filter_masks.len()>10_000 || state.filter_stacks.len()>10_000 || state.item_sets.len()>10_000 || state.opaque.len()>100_000 {return Err(invalid("item_limit"));}
    let mut references=state.items.len().checked_add(state.source_items.len()).and_then(|n|n.checked_add(state.opaque.len())).and_then(|n|n.checked_add(state.selection.selected.len())).and_then(|n|n.checked_add(state.selection.selected_paths.len())).filter(|n|*n<=100_000).ok_or_else(||invalid("item_limit"))?;
    for set in &state.item_sets{poll(control)?;references=references.checked_add(set.members.len()).filter(|n|*n<=100_000).ok_or_else(||invalid("item_limit"))?;}
    for stack in &state.filter_stacks{poll(control)?;references=references.checked_add(stack.nodes.len()).filter(|n|*n<=100_000).ok_or_else(||invalid("item_limit"))?;for node in &stack.nodes{poll(control)?;references=references.checked_add(node.auxiliary.len()).filter(|n|*n<=100_000).ok_or_else(||invalid("item_limit"))?;}}
    let mut channel=None;let mut has_source=false;
    for value in &doc.channels{poll(control)?;if value.id==target{channel=Some(value);}has_source|=value.id==source;}
    let channel=channel.filter(|_|has_source).ok_or_else(||invalid("clone_channel"))?;
    if channel.name.len()>1<<20{return Err(invalid("string_limit"));}for chunk in channel.name.as_bytes().chunks(32<<10){poll(control)?;if chunk.contains(&0){return Err(invalid("string_limit"));}}
    let from=ItemRef::Channel(source);let to=ItemRef::Channel(target);
    for item in &state.items{poll(control)?;if item.item==to{return Err(invalid("clone_channel"));}}
    // The canonical validator permits the already-added native target without
    // an ItemState. All unchanged references/resources/source payloads validate
    // once; append-only rewrites below cannot alter their semantics.
    state.validate_with(doc,control)?;
    let original=state.items.iter().find(|item|item.item==from).ok_or_else(||invalid("clone_channel"))?;
    let mut nodes=0usize;let mut masks=0usize;let mut stacks=0usize;let mut max_mask=0u64;
    for item in &state.items{poll(control)?;if let ItemRef::Mask(id)=item.item{max_mask=max_mask.max(id.0);}}
    for stack in &state.filter_stacks{poll(control)?;if stack.owner==from{stacks+=1;nodes=nodes.checked_add(stack.nodes.len()).ok_or_else(||invalid("item_limit"))?;for node in &stack.nodes{poll(control)?;masks+=usize::from(matches!(node.mask,Some(ItemRef::Mask(_))));}}}
    if nodes>1024 || masks>1024 || stacks>1{return Err(invalid("filter_limit"));}
    let mut total_filters=nodes;
    for stack in &state.filter_stacks{poll(control)?;total_filters=total_filters.checked_add(stack.nodes.len()).filter(|n|*n<=100_000).ok_or_else(||invalid("filter_identity"))?;}
    let count_items=state.items.len().checked_add(1).and_then(|n|n.checked_add(masks)).filter(|n|*n<=10_000).ok_or_else(||invalid("item_limit"))?;
    let count_stacks=state.filter_stacks.len().checked_add(stacks).filter(|n|*n<=10_000).ok_or_else(||invalid("item_limit"))?;
    let count_masks=state.filter_masks.len().checked_add(masks).filter(|n|*n<=10_000).ok_or_else(||invalid("item_limit"))?;
    let arc_count=state.resources.len().checked_add(state.source_items.len()).and_then(|n|n.checked_add(state.opaque.len())).and_then(|n|n.checked_add(state.editable_indexed.len().checked_mul(2)?)).and_then(|n|state.filter_masks.iter().try_fold(n,|n,mask|n.checked_add(mask.surface.tile_count()))).ok_or_else(||invalid("clone_channel_limit"))?;
    if (arc_count as u64).checked_mul(96).is_none_or(|n|n>16<<20){return Err(invalid("clone_channel_limit"));}
    let mut seen=HashSet::new();seen.try_reserve(arc_count).map_err(|_|invalid("clone_channel_limit"))?;
    let index_bytes=(seen.capacity() as u64).checked_mul(48).ok_or_else(||invalid("clone_channel_limit"))?;
    if index_bytes>16<<20{return Err(invalid("clone_channel_limit"));}
    let mut census=Census{control,model:index_bytes,payload:0,work:0,visits:0,seen};
    census.bytes(size_of::<XcfState>() as u64)?;
    census.bytes((filter_ids.len() as u64).checked_mul(64).ok_or_else(||invalid("clone_channel_limit"))?)?;
    census.array(&state.items)?;for value in &state.items{census.item(value)?;}
    census.array(&state.resources)?;for resource in &state.resources{census.text(&resource.name)?;for value in [&resource.collection,&resource.locator,&resource.content_hash].into_iter().flatten(){census.text(value)?;}if let Some(resolution)=&resource.resolution{census.text(&resolution.payload_hash)?;}if let Some(bytes)=&resource.owned_bytes{census.arc(bytes)?;}}
    census.array(&state.source_items)?;for source in &state.source_items{poll(control)?;match &source.source{EditableSource::Text{content,markup,configuration,..}=>{census.text(content)?;if let Some(markup)=markup{census.text(markup)?;}census.arguments(configuration,0)?;},EditableSource::Vector{configuration,..}=>census.arguments(configuration,0)?,EditableSource::Linked{last_verified_hash,configuration,..}=>{if let Some(hash)=last_verified_hash{census.text(hash)?;}census.arguments(configuration,0)?;}}if let Some(cache)=&source.cache{census.text(&cache.blake3)?;census.arc(&cache.bytes)?;}}
    census.array(&state.filter_stacks)?;for stack in &state.filter_stacks{census.stack(stack)?;}
    census.array(&state.filter_masks)?;for mask in &state.filter_masks{census.surface(&mask.surface)?;}
    census.array(&state.editable_indexed)?;for plane in &state.editable_indexed{census.arc(plane.raster.indices())?;if let Some(alpha)=plane.raster.alpha(){census.arc(alpha)?;}census.bytes(plane.raster.palette_capacity_bytes() as u64)?;}
    census.array(&state.selection.selected)?;census.array(&state.selection.selected_paths)?;census.array(&state.item_sets)?;for set in &state.item_sets{census.text(&set.name)?;match &set.rule{crate::xcf_editing::ItemSetRule::Text(value)|crate::xcf_editing::ItemSetRule::Regex(value)|crate::xcf_editing::ItemSetRule::Glob(value)=>census.text(value)?,_=>{}}census.array(&set.members)?;}
    census.array(&state.opaque)?;let mut logical_opaque=0u64;for value in &state.opaque{census.opaque(value)?;add(&mut logical_opaque,value.bytes.len() as u64)?;}
    for value in [&state.provenance.format,&state.provenance.container,&state.provenance.source_label]{census.text(value)?;}for value in [&state.provenance.producer,&state.provenance.release_id,&state.provenance.source_hash,&state.provenance.original_locator].into_iter().flatten(){census.text(value)?;}census.array(&state.provenance.protected_source_fingerprints)?;for value in &state.provenance.protected_source_fingerprints{census.text(value)?;}
    census.item(original)?;census.bytes(channel.name.len() as u64)?;
    // All selected IDs, masks and typed relationships are resolved before any
    // record clone. Bounds/flags/raw graph/resource IDs retain original meaning.
    let plan_bytes=(nodes as u64).checked_mul(size_of::<(FilterId,FilterId)>() as u64).and_then(|n|n.checked_add((masks as u64).checked_mul(size_of::<(MaskId,MaskId,FilterId)>() as u64)?)).ok_or_else(||invalid("clone_channel_limit"))?;
    census.bytes(plan_bytes)?;
    if census.model.checked_add(census.payload).is_none_or(|n|n>photocraft_raster::MAX_CHECKED_RASTER_BYTES){return Err(invalid("clone_channel_limit"));}
    let mut selected=reserved::<(FilterId,FilterId)>(nodes)?;let mut mask_map=reserved::<(MaskId,MaskId,FilterId)>(masks)?;
    for stack in state.filter_stacks.iter().filter(|stack|stack.owner==from){
        census.stack(stack)?;
        for node in &stack.nodes{
            poll(control)?;let id=filter_ids.get(&node.id).copied().filter(|id|id.0!=0).ok_or_else(||invalid("clone_filter_id"))?;
            if selected.iter().any(|(_,fresh)|*fresh==id){return Err(invalid("clone_filter_id"));}
            for existing in &state.filter_stacks{poll(control)?;for node in &existing.nodes{poll(control)?;if node.id==id{return Err(invalid("clone_filter_id"));}}}
            selected.push((node.id,id));
            if let Some(ItemRef::Mask(old))=node.mask {
                let mask=state.filter_masks.iter().find(|mask|mask.id==old && mask.owner==node.id).ok_or_else(||invalid("clone_channel_mask"))?;
                let metadata=state.items.iter().find(|item|item.item==ItemRef::Mask(old)).ok_or_else(||invalid("clone_channel_mask"))?;
                max_mask=max_mask.checked_add(1).ok_or_else(||invalid("clone_mask_id"))?;
                if mask_map.iter().any(|(seen,_,_)|*seen==old){return Err(invalid("clone_channel_mask"));}
                mask_map.push((old,MaskId(max_mask),id));census.item(metadata)?;census.surface(&mask.surface)?;
            }
        }
    }
    let mut extra_opaque=0usize;
    for value in &state.opaque{poll(control)?;if value.owner==Some(from) || matches!(value.owner,Some(ItemRef::Mask(id)) if mask_map.iter().any(|(old,_,_)|*old==id)){extra_opaque+=1;census.opaque(value)?;add(&mut logical_opaque,value.bytes.len() as u64)?;}}
    let count_opaque=state.opaque.len().checked_add(extra_opaque).filter(|n|*n<=100_000).ok_or_else(||invalid("opaque_limit"))?;
    if logical_opaque>256<<20{return Err(invalid("opaque_limit"));}
    let replacement_bytes=(count_items as u64).checked_mul(size_of::<ItemState>() as u64).and_then(|n|n.checked_add((count_stacks as u64).checked_mul(size_of::<FilterStack>() as u64)?)).and_then(|n|n.checked_add((count_masks as u64).checked_mul(size_of::<FilterMask>() as u64)?)).and_then(|n|n.checked_add((count_opaque as u64).checked_mul(size_of::<OpaqueRecord>() as u64)?)).ok_or_else(||invalid("clone_channel_limit"))?;
    census.bytes(replacement_bytes)?;
    census.bytes(((1+masks) as u64).checked_mul(size_of::<ItemState>() as u64).and_then(|n|n.checked_add((stacks as u64).checked_mul(size_of::<FilterStack>() as u64)?)).and_then(|n|n.checked_add((masks as u64).checked_mul(size_of::<FilterMask>() as u64)?)).and_then(|n|n.checked_add((extra_opaque as u64).checked_mul(size_of::<OpaqueRecord>() as u64)?)).ok_or_else(||invalid("clone_channel_limit"))?)?;
    add(&mut census.work,(nodes as u64).checked_mul(100_000).and_then(|n|n.checked_add((extra_opaque as u64).checked_mul(masks as u64)?)).ok_or_else(||invalid("clone_channel_limit"))?)?;
    if census.model.checked_add(census.payload).is_none_or(|n|n>photocraft_raster::MAX_CHECKED_RASTER_BYTES) || census.work>8_000_000_000{return Err(invalid("clone_channel_limit"));}
    let mut new_items=reserved::<ItemState>(count_items)?;let mut new_stacks=reserved::<FilterStack>(count_stacks)?;let mut new_masks=reserved::<FilterMask>(count_masks)?;let mut new_opaque=reserved::<OpaqueRecord>(count_opaque)?;
    let mut pending_items=reserved(1+masks)?;let mut pending_stacks=reserved(stacks)?;let mut pending_masks=reserved(masks)?;let mut pending_opaque=reserved(extra_opaque)?;
    let mut copy=item(original,control)?;copy.item=to;copy.name=string(&channel.name,control)?;copy.custom_name=true;pending_items.push(copy);
    for stack in state.filter_stacks.iter().filter(|stack|stack.owner==from){
        let mut new_nodes=reserved(stack.nodes.len())?;
        for original in &stack.nodes{
            poll(control)?;let mut copy=node(original,control)?;copy.id=selected.iter().find(|(old,_)|*old==original.id).map(|(_,new)|*new).ok_or_else(||invalid("clone_filter_id"))?;
            if let Some(ItemRef::Mask(old))=original.mask{
                let (_,fresh,owner)=mask_map.iter().find(|(seen,_,_)|*seen==old).copied().ok_or_else(||invalid("clone_channel_mask"))?;
                let mask=state.filter_masks.iter().find(|mask|mask.id==old && mask.owner==original.id).ok_or_else(||invalid("clone_channel_mask"))?;
                pending_masks.push(FilterMask{id:fresh,owner,surface:surface(&mask.surface,control)?,bounds:mask.bounds});copy.mask=Some(ItemRef::Mask(fresh));
                let metadata=state.items.iter().find(|item|item.item==ItemRef::Mask(old)).ok_or_else(||invalid("clone_channel_mask"))?;
                let mut copied=item(metadata,control)?;copied.item=ItemRef::Mask(fresh);if let Some(mask)=&mut copied.mask{if mask.owner==from{mask.owner=to;}}pending_items.push(copied);
            }
            for (_,input) in &mut copy.auxiliary{poll(control)?;if *input==from{*input=to;}}
            new_nodes.push(copy);
        }
        pending_stacks.push(FilterStack{owner:to,nodes:new_nodes,revision:stack.revision});
    }
    for value in &state.opaque{poll(control)?;let owner=match value.owner{Some(owner) if owner==from=>Some(to),Some(ItemRef::Mask(id))=>mask_map.iter().find(|(old,_,_)|*old==id).map(|(_,fresh,_)|ItemRef::Mask(*fresh)),_=>None};if let Some(owner)=owner{pending_opaque.push(OpaqueRecord{owner:Some(owner),namespace:string(&value.namespace,control)?,identity:string(&value.identity,control)?,source_version:value.source_version,order:value.order,flags:value.flags,bytes:value.bytes.clone(),blake3:string(&value.blake3,control)?,rendering_required:value.rendering_required});}}
    // Baseline validated once. New ItemRefs/FilterIds/MaskIds are unique and
    // either unchanged-valid or rewritten to explicitly staged records. Scalar
    // configuration, resource bindings, hashes and current precision are exact
    // copies; prospective cumulative nested-config and opaque quotas were charged.
    poll(control)?;
    new_items.append(&mut state.items);new_items.append(&mut pending_items);
    new_stacks.append(&mut state.filter_stacks);new_stacks.append(&mut pending_stacks);
    new_masks.append(&mut state.filter_masks);new_masks.append(&mut pending_masks);
    new_opaque.append(&mut state.opaque);new_opaque.append(&mut pending_opaque);
    state.items=new_items;state.filter_stacks=new_stacks;state.filter_masks=new_masks;state.opaque=new_opaque;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AlphaChannel,ColorMode,PixelFormat,SampleType,Size,xcf_state::{ItemLocks,MaskState}};
    fn fixture()->(Document,XcfState,ChannelId,ChannelId) {
        let mut doc=Document::new("Own metadata clone",Size::new(2,2),ColorMode::Rgb,SampleType::F32);
        let gray=PixelFormat{mode:ColorMode::Grayscale,sample:SampleType::F32,alpha:false};
        let source=AlphaChannel::new("Own source",Surface::new(gray));let from=source.id;doc.channels.push(source);
        let mut state=XcfState::default();state.reconcile_layers(&doc).unwrap();
        state.items.iter_mut().find(|item|item.item==ItemRef::Channel(from)).unwrap().source_tattoo=Some(42);
        let bounds=crate::Rect::new(0,0,1,1);
        let mut surface=Surface::try_with_default_bytes(gray,&0.25_f32.to_le_bytes()).unwrap();
        surface.try_write_interleaved_with(crate::Rect::new(-1,-1,0,0),&0.75_f32.to_le_bytes(),&Interrupt::NONE).unwrap();
        state.filter_masks.push(FilterMask{id:MaskId(20),owner:FilterId(10),surface,bounds});
        state.items.push(ItemState{item:ItemRef::Mask(MaskId(20)),name:"Own mask".into(),custom_name:false,visible:true,locks:ItemLocks::default(),parent:None,source_tattoo:Some(72),source_flags:29,raster_bounds:Some(bounds),mask:Some(MaskState{owner:ItemRef::Channel(from),bounds,offset:[0,0],outside_f64_bits:0.25_f64.to_bits(),enabled:true,shown:false,editing:false,linked:true}),composition:None});
        state.filter_stacks.push(FilterStack{owner:ItemRef::Channel(from),revision:7,nodes:vec![FilterNode{id:FilterId(10),operation:OperationIdentity{name:"own:metadata".into(),stored_version:"0:0".into(),schema_epoch:"own-only".into()},source_display_name:Some("Own source display".into()),source_icon:None,arguments:vec![FilterArgument{name:"config".into(),value:FilterValue::Config(vec![FilterArgument{name:"signed-zero".into(),value:FilterValue::Number(ExactNumber::Float64Bits((-0.0_f64).to_bits()))}])}],enabled:true,opacity_f64_bits:1.0_f64.to_bits(),source_clipping:Some(0),source_region:Some(0),mask:Some(ItemRef::Mask(MaskId(20))),auxiliary:vec![("self".into(),ItemRef::Channel(from))],composition:None}]});
        let bytes=Arc::new(vec![0,255,2]);
        state.opaque.push(OpaqueRecord{owner:Some(ItemRef::Mask(MaskId(20))),namespace:"own".into(),identity:"mask record".into(),source_version:1,order:3,flags:17,blake3:blake3::hash(&bytes).to_hex().to_string(),bytes,rendering_required:false});
        let target=AlphaChannel::new("Own copy",Surface::new(gray));let to=target.id;doc.channels.push(target);
        state.validate(&doc).unwrap();(doc,state,from,to)
    }
    #[test]
    fn controlled_clone_preserves_owned_bits_and_remaps_only_the_existing_contract() {
        let (doc,mut state,from,to)=fixture();let before=state.clone();
        state.clone_channel_metadata_with_interrupt(from,to,&doc,&BTreeMap::from([(FilterId(10),FilterId(30))]),&Interrupt::NONE).unwrap();
        state.validate(&doc).unwrap();
        let stack=state.filter_stacks.last().unwrap();assert_eq!(stack.owner,ItemRef::Channel(to));assert_eq!(stack.nodes[0].id,FilterId(30));assert_eq!(stack.nodes[0].auxiliary[0].1,ItemRef::Channel(to));assert_eq!(stack.nodes[0].arguments,before.filter_stacks[0].nodes[0].arguments);
        let mask=state.filter_masks.last().unwrap();assert_eq!(mask.id,MaskId(21));assert_eq!(mask.owner,FilterId(30));assert_eq!(mask.surface,before.filter_masks[0].surface);
        assert!(Arc::ptr_eq(&state.opaque.last().unwrap().bytes,&before.opaque[0].bytes));
        assert_eq!(state.items.iter().find(|item|item.item==ItemRef::Channel(to)).unwrap().source_tattoo,Some(42));
        assert_eq!(state.selection,before.selection);assert_eq!(state.source_items,before.source_items);assert_eq!(state.resources,before.resources);
    }
    #[test]
    fn late_staged_mask_cancel_and_invalid_map_are_atomic() {
        let (doc,mut state,from,to)=fixture();let before=state.clone();
        assert!(state.clone_channel_metadata_with_interrupt(from,to,&doc,&BTreeMap::new(),&Interrupt::NONE).is_err());assert_eq!(state,before);
        let tile=state.filter_masks[0].surface.tiles().next().unwrap().1.clone();let initial=Arc::strong_count(&tile);
        let cancel=||Arc::strong_count(&tile)>initial;let control=Interrupt::cancel_only(&cancel);
        assert_eq!(state.clone_channel_metadata_with_interrupt(from,to,&doc,&BTreeMap::from([(FilterId(10),FilterId(30))]),&control).unwrap_err().code,"cancelled");
        assert_eq!(state,before);
    }
}
