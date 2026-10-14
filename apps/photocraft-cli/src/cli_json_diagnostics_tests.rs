//! Unrun implementation drafts: real CLI routes, shared failure reports and
//! argument ordering. No source-fidelity or interface acceptance is claimed.
use super::*;
fn invoke(args:&[&str])->(i32,String,String){let args=args.iter().map(|s|s.to_string()).collect::<Vec<_>>();let mut out=Vec::new();let mut err=Vec::new();let code=run(&args,&mut out,&mut err);(code,String::from_utf8(out).unwrap(),String::from_utf8(err).unwrap())}
fn folder()->PathBuf{static NEXT:std::sync::atomic::AtomicU64=std::sync::atomic::AtomicU64::new(0);let dir=std::env::temp_dir().join(format!("photocraft-cli-json-{}-{}",std::process::id(),NEXT.fetch_add(1,std::sync::atomic::Ordering::Relaxed)));std::fs::create_dir(&dir).unwrap();dir}
#[test]
fn command_and_flag_syntax_precedes_nonexistent_input_io(){
    let(code,out,err)=invoke(&["run","never-open-this-input.xcf","--cmd","layer.select","--params","{","--json-errors"]);assert_eq!(code,1);assert!(out.is_empty());let reply:Value=serde_json::from_str(&err).unwrap();assert!(reply["error"].as_str().unwrap().contains("bad --params JSON"));assert!(reply.get("errorDetails").is_none());
    let(code,_,err)=invoke(&["run","never-open-this-input.xcf","--cmd","layer.select","--unknown","--json-errors"]);assert_eq!(code,2);let reply:Value=serde_json::from_str(&err).unwrap();assert!(reply["error"].as_str().unwrap().contains("unknown flag"));
    let(code,_,err)=invoke(&["run","never-open-this-input.xcf","--cmd","layer.select","--params","{"]);assert_eq!(code,1);assert!(err.starts_with("error: bad --params JSON"));
}
#[test]
fn info_convert_run_keep_shared_xcf_failure_details_and_no_output(){
    let dir=folder();let input=dir.join("unsupported.xcf");let output=dir.join("result.pcraft");std::fs::write(&input,b"gimp xcf v999\0").unwrap();let input=input.to_str().unwrap();let output_name=output.to_str().unwrap();
    let expected=files::open(Path::new(input)).err().unwrap().error_reply()["errorDetails"].clone();
    for args in [vec!["info",input,"--json-errors"],vec!["convert",input,output_name,"--json-errors"],vec!["run",input,"--cmd","layer.select","--params","{}","--json-errors"]]{
        let(code,out,err)=invoke(&args);assert_eq!(code,1);assert!(out.is_empty());let reply:Value=serde_json::from_str(&err).unwrap();assert_eq!(reply["ok"],false);assert_eq!(reply["errorDetails"],expected);assert!(!output.exists());
    }
    let(code,_,err)=invoke(&["info",input]);assert_eq!(code,1);assert!(err.starts_with("error: "));std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn convert_json_is_opt_in_and_uses_existing_assessment_without_inventing_fidelity(){
    let dir=folder();let input=dir.join("source.pcraft");let output=dir.join("result.pcraft");let second=dir.join("legacy.pcraft");let mut session=Headless::trusted_local();session.command_run("file.new",json!({"width":2,"height":2})).unwrap();session.save(None,Some(&input),None,&ExportOptions::default()).unwrap();
    let(code,out,_)=invoke(&["convert",input.to_str().unwrap(),output.to_str().unwrap(),"--json"]);assert_eq!(code,0);let reply:Value=serde_json::from_str(&out).unwrap();assert_eq!(reply["ok"],true);assert_eq!(reply["format"],"pcraft");assert!(reply["assessment"].is_null());assert!(reply["warnings"].is_array());
    let(code,out,_)=invoke(&["convert",input.to_str().unwrap(),second.to_str().unwrap()]);assert_eq!(code,0);assert!(out.is_empty());std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn batch_json_errors_are_bound_to_each_file_and_not_the_aggregate(){
    let dir=folder();let input=dir.join("inputs");let output=dir.join("outputs");std::fs::create_dir(&input).unwrap();let actions=dir.join("actions.json");std::fs::write(&actions,b"[]").unwrap();std::fs::write(input.join("a.xcf"),b"gimp xcf v999\0").unwrap();std::fs::write(input.join("b.png"),b"not a PNG").unwrap();
    let(code,out,err)=invoke(&["batch","--actions",actions.to_str().unwrap(),"--in",input.to_str().unwrap(),"--out",output.to_str().unwrap(),"--format","pcraft","--json-errors"]);assert_eq!(code,1);assert!(out.contains("0 succeeded, 2 failed"));
    let replies=err.lines().filter(|line|line.starts_with('{')).map(|line|serde_json::from_str::<Value>(line).unwrap()).collect::<Vec<_>>();assert_eq!(replies.len(),3);assert!(replies[0]["file"].as_str().unwrap().ends_with("a.xcf"));assert_eq!(replies[0]["errorDetails"]["sourceFormat"],"xcf");assert!(replies[1]["file"].as_str().unwrap().ends_with("b.png"));assert!(replies[1].get("errorDetails").is_none());assert!(replies[2].get("file").is_none());assert!(replies[2].get("errorDetails").is_none());assert!(!output.join("a.pcraft").exists());assert!(!output.join("b.pcraft").exists());std::fs::remove_dir_all(dir).unwrap();
}
#[test]
fn batch_export_and_shape_errors_precede_actions_read_and_output_creation(){
    let dir=folder();let actions=dir.join("not-present.json");let output=dir.join("not-created");
    let(code,_,err)=invoke(&["batch","--actions",actions.to_str().unwrap(),"--in",dir.to_str().unwrap(),"--out",output.to_str().unwrap(),"--quality","bad","--json-errors"]);assert_eq!(code,1);let reply:Value=serde_json::from_str(&err).unwrap();assert!(reply["error"].as_str().unwrap().contains("bad --quality"));assert!(!output.exists());
    let(code,_,err)=invoke(&["batch","unexpected-positional","--actions",actions.to_str().unwrap(),"--in",dir.to_str().unwrap(),"--out",output.to_str().unwrap(),"--json-errors"]);assert_eq!(code,1);let reply:Value=serde_json::from_str(&err).unwrap();assert!(reply["error"].as_str().unwrap().contains("positional"));assert!(!output.exists());std::fs::remove_dir_all(dir).unwrap();
}
