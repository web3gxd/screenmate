//! A private fake app and temporary LOCALAPPDATA exercise the complete
//! daemon/stdio path. No screen or user configuration is accessed.
#![cfg(windows)]
use serde_json::{json,Value};
use std::{fs,io::{BufRead,BufReader,Read,Write},net::{Ipv4Addr,SocketAddrV4,TcpListener,TcpStream},os::windows::{ffi::OsStrExt,process::CommandExt},path::{Path,PathBuf},process::{Child,Command,Stdio},sync::atomic::{AtomicUsize,Ordering},thread,time::{Duration,Instant}};

const NO_WINDOW:u32=0x08000000;
static COUNTER:AtomicUsize=AtomicUsize::new(0);
fn binary()->&'static str {env!("CARGO_BIN_EXE_screenmate-mcp")}
fn cmd(root:&Path)->Command {let mut c=Command::new(binary());c.env("LOCALAPPDATA",root).creation_flags(NO_WINDOW);c}

struct TestRoot {path:PathBuf,parent:PathBuf}
impl TestRoot {
    fn new()->Self {
        let parent=PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("daemon-integration-tests");
        fs::create_dir_all(&parent).unwrap();
        let path=parent.join(format!("{}-{}",std::process::id(),COUNTER.fetch_add(1,Ordering::Relaxed)));
        fs::create_dir(&path).unwrap();Self{path,parent}
    }
}
impl Drop for TestRoot {fn drop(&mut self) {
    // Delete only the dedicated test directory after validating its location.
    if let (Ok(path),Ok(parent))=(fs::canonicalize(&self.path),fs::canonicalize(&self.parent)) {
        if path.parent()==Some(parent.as_path()) {let _=fs::remove_dir_all(path);}
    }
}}
struct ChildGuard(Child);
impl Drop for ChildGuard {fn drop(&mut self){if self.0.try_wait().ok().flatten().is_none(){let _=self.0.kill();let _=self.0.wait();}}}

fn wait_exit(child:&mut Child,timeout:Duration)->std::process::ExitStatus {
    let until=Instant::now()+timeout;
    loop {if let Some(status)=child.try_wait().unwrap(){return status;}if Instant::now()>=until {let _=child.kill();panic!("child did not exit within timeout");}thread::sleep(Duration::from_millis(20));}
}
fn wait_metadata(root:&Path,daemon:&mut Child)->Value {
    let file=root.join("Screenmate").join("mcp").join("daemon.json");let until=Instant::now()+Duration::from_secs(8);
    loop {
        if let Ok(bytes)=fs::read(&file) {if let Ok(value)=serde_json::from_slice::<Value>(&bytes){if value["pid"].as_u64()==Some(daemon.id() as u64){return value;}}}
        if let Some(status)=daemon.try_wait().unwrap(){panic!("daemon exited before it was ready: {status}");}
        if Instant::now()>=until {panic!("daemon metadata did not appear");}thread::sleep(Duration::from_millis(25));
    }
}
fn line(stream:&mut impl BufRead)->Vec<u8> {let mut value=Vec::new();stream.read_until(b'\n',&mut value).unwrap();value}
fn daemon_auth(info:&Value,token:&str)->Value {
    let addr=SocketAddrV4::new(Ipv4Addr::LOCALHOST,info["port"].as_u64().unwrap() as u16);
    let mut stream=TcpStream::connect_timeout(&addr.into(),Duration::from_secs(2)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    writeln!(stream,"{}",json!({"token":token})).unwrap();
    serde_json::from_slice(&line(&mut BufReader::new(stream))).unwrap()
}
fn authorised_socket(info:&Value,token:&str)->TcpStream {
    let addr=SocketAddrV4::new(Ipv4Addr::LOCALHOST,info["port"].as_u64().unwrap() as u16);
    let mut stream=TcpStream::connect_timeout(&addr.into(),Duration::from_secs(2)).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();stream.set_write_timeout(Some(Duration::from_secs(2))).unwrap();
    writeln!(stream,"{}",json!({"token":token})).unwrap();
    let ack:Value=serde_json::from_slice(&line(&mut BufReader::new(stream.try_clone().unwrap()))).unwrap();assert_eq!(ack["ok"],true);
    stream
}

/// Inspect the Windows DACL as SDDL without displaying it or any secrets.
fn dacl(path:&Path)->String {
    use std::ffi::c_void;
    #[link(name="advapi32")] extern "system" {
        fn GetNamedSecurityInfoW(name:*mut u16,object_type:u32,information:u32,owner:*mut *mut c_void,group:*mut *mut c_void,dacl:*mut *mut c_void,sacl:*mut *mut c_void,descriptor:*mut *mut c_void)->u32;
        fn ConvertSecurityDescriptorToStringSecurityDescriptorW(descriptor:*const c_void,revision:u32,information:u32,text:*mut *mut u16,length:*mut u32)->i32;
    }
    #[link(name="kernel32")] extern "system" {fn LocalFree(memory:isize)->isize;}
    let mut name:Vec<u16>=path.as_os_str().encode_wide().chain(Some(0)).collect();let mut descriptor=std::ptr::null_mut();let mut acl=std::ptr::null_mut();
    assert_eq!(unsafe{GetNamedSecurityInfoW(name.as_mut_ptr(),1,4,std::ptr::null_mut(),std::ptr::null_mut(),&mut acl,std::ptr::null_mut(),&mut descriptor)},0);
    let mut text=std::ptr::null_mut();let mut size=0;
    assert_ne!(unsafe{ConvertSecurityDescriptorToStringSecurityDescriptorW(descriptor,1,4,&mut text,&mut size)},0);
    let result=String::from_utf16_lossy(unsafe{std::slice::from_raw_parts(text,size.saturating_sub(1) as usize)});
    unsafe{LocalFree(text as isize);LocalFree(descriptor as isize);}result
}
fn current_sid()->String {
    let out=Command::new("whoami").args(["/user","/fo","csv","/nh"]).creation_flags(NO_WINDOW).output().unwrap();assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).trim().split(',').nth(1).unwrap().trim().trim_matches('"').to_owned()
}

fn send_all_and_close(root:&Path,requests:&[Value],secrets:&[&str])->Vec<Value> {
    let mut child=cmd(root).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    {let mut input=child.stdin.take().unwrap();for request in requests {writeln!(input,"{request}").unwrap();}}
    let status=wait_exit(&mut child,Duration::from_secs(8));
    let mut stderr=String::new();child.stderr.take().unwrap().read_to_string(&mut stderr).unwrap();
    for secret in secrets {stderr=stderr.replace(secret,"[redacted]");}
    assert!(status.success(),"stdio bridge failed for {} requests: {status}: {stderr}",requests.len());
    let mut output=String::new();child.stdout.take().unwrap().read_to_string(&mut output).unwrap();
    for secret in secrets {assert!(!output.contains(secret),"a secret was written to stdout");}
    output.lines().map(|line|serde_json::from_str(line).expect("stdout must contain only JSON-RPC")).collect()
}

#[test]
fn resident_auth_stdio_eof_shutdown_restart_and_stale_fallback() {
    let root=TestRoot::new();
    let mut daemon=ChildGuard(cmd(&root.path).arg("--daemon").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
    let info=wait_metadata(&root.path,&mut daemon.0);
    let token=info["token"].as_str().unwrap().to_owned();assert_eq!(token.len(),64);
    assert_eq!(info["version"],1);
    let folder=root.path.join("Screenmate").join("mcp");let meta=folder.join("daemon.json");
    let directory_acl=dacl(&folder);let file_acl=dacl(&meta);let sid=current_sid();
    assert!(directory_acl.starts_with("D:P"),"daemon directory DACL must be protected");
    assert!(directory_acl.contains(&sid) && file_acl.contains(&sid),"directory and file must grant current user access");
    assert_eq!(directory_acl.matches("(A;").count(),1,"directory must have only one allow ACE");
    assert_eq!(file_acl.matches("(A;").count(),1,"metadata must have only one allow ACE");
    assert_eq!(daemon_auth(&info,&"0".repeat(64))["ok"],false);
    assert_eq!(daemon_auth(&info,&token)["ok"],true);
    // A single oversized request closes only this connection, not the daemon.
    let mut oversized=authorised_socket(&info,&token);
    let _=oversized.write_all(&vec![b'x';1024*1024+2]);
    let mut one=[0u8;1];let result=oversized.read(&mut one);
    assert!(matches!(result,Ok(0)|Err(_)),"oversized request must be rejected");
    assert_eq!(daemon_auth(&info,&token)["ok"],true);

    // A duplicate daemon exits instead of starting another listener.
    let mut duplicate=cmd(&root.path).arg("--daemon").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    assert!(wait_exit(&mut duplicate,Duration::from_secs(3)).success());
    assert!(daemon.0.try_wait().unwrap().is_none());

    // Discovery remains usable while the separate GUI application is absent.
    let requests_without_app=[json!({"jsonrpc":"2.0","id":20,"method":"tools/list"}),json!({"jsonrpc":"2.0","id":21,"method":"tools/call","params":{"name":"look","arguments":{}}})];
    let replies=send_all_and_close(&root.path,&requests_without_app,&[&token]);
    assert_eq!(replies.len(),2);
    assert_eq!(replies.iter().find(|v|v["id"]==21).unwrap()["result"]["isError"],true);

    // Fake application server: capture is a tiny constant and never a screen.
    let listener=TcpListener::bind((Ipv4Addr::LOCALHOST,0)).unwrap();let app_port=listener.local_addr().unwrap().port();
    let app_token="b".repeat(64);let expected=app_token.clone();
    let app=thread::spawn(move || {
        let (mut stream,_)=listener.accept().unwrap();stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut r=BufReader::new(stream.try_clone().unwrap());let mut headers=String::new();let mut len=0usize;let mut authorised=false;
        loop {headers.clear();r.read_line(&mut headers).unwrap();if headers=="\r\n"{break;}if let Some((name,value))=headers.split_once(':') {if name.eq_ignore_ascii_case("content-length"){len=value.trim().parse().unwrap();}if name.eq_ignore_ascii_case("authorization"){authorised=value.trim()==format!("Bearer {expected}");}}}
        assert!(authorised);let mut body=vec![0;len];r.read_exact(&mut body).unwrap();
        let request:Value=serde_json::from_slice(&body).unwrap();assert_eq!(request["method"],"look");
        let body=json!({"ok":true,"result":{"image_base64":"/9j/2Q==","mime_type":"image/jpeg","timestamp":"test","width":4,"height":2,"changed":true}}).to_string();
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body).unwrap();
    });
    fs::create_dir_all(root.path.join("Screenmate")).unwrap();
    fs::write(root.path.join("Screenmate").join("session.json"),json!({"port":app_port,"token":app_token}).to_string()).unwrap();
    let requests=[json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}),json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"look","arguments":{}}})];
    // stdin is closed immediately; the bridge must still drain all replies.
    let replies=send_all_and_close(&root.path,&requests,&[&token,&app_token]);app.join().unwrap();
    assert_eq!(replies.len(),3);
    let capture=replies.iter().find(|v|v["id"]==3).unwrap();assert_eq!(capture["result"]["content"][1]["type"],"image");

    // A bridge with stdin still open must terminate if its daemon disappears;
    // otherwise Codex would hang indefinitely on the next request.
    let mut held=ChildGuard(cmd(&root.path).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap());
    let mut input=held.0.stdin.take().unwrap();
    writeln!(input,"{}",json!({"jsonrpc":"2.0","id":8,"method":"ping"})).unwrap();
    let output=held.0.stdout.take().unwrap();let (tx,rx)=std::sync::mpsc::sync_channel(1);
    thread::spawn(move || {let mut r=BufReader::new(output);let _=tx.send(line(&mut r));});
    let ping:Value=serde_json::from_slice(&rx.recv_timeout(Duration::from_secs(3)).expect("bridge ping timed out")).unwrap();assert_eq!(ping["id"],8);
    let mut stop=cmd(&root.path).arg("--stop").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    assert!(wait_exit(&mut stop,Duration::from_secs(3)).success());
    assert!(wait_exit(&mut daemon.0,Duration::from_secs(3)).success());
    assert!(!wait_exit(&mut held.0,Duration::from_secs(3)).success(),"bridge must detect lost daemon");drop(input);
    assert!(!meta.exists(),"graceful stop must remove discovery metadata");

    // An orphaned file cannot break stdio discovery or start a daemon silently.
    fs::write(&meta,info.to_string()).unwrap();
    let mut stale_stop=cmd(&root.path).arg("--stop").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    assert!(!wait_exit(&mut stale_stop,Duration::from_secs(3)).success(),"stale metadata must not report a successful stop");
    let replies=send_all_and_close(&root.path,&requests[..2],&[&token]);
    assert_eq!(replies.len(),2);assert!(replies.iter().any(|v|v["id"]==2));

    let mut restarted=ChildGuard(cmd(&root.path).arg("--daemon").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
    let new_info=wait_metadata(&root.path,&mut restarted.0);
    assert!(new_info["token"]!=info["token"],"daemon restart must rotate its token");
    let mut stop=cmd(&root.path).arg("--stop").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
    assert!(wait_exit(&mut stop,Duration::from_secs(3)).success());assert!(wait_exit(&mut restarted.0,Duration::from_secs(3)).success());assert!(!meta.exists());
}
