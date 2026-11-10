//! Local authenticated daemon and stdio bridge. The daemon never captures the
//! screen itself: the existing app RPC applies masks and pause checks.
use serde_json::{json,Value};
use std::{env,fs::{self,File,OpenOptions},hash::{Hash,Hasher},io::{self,BufRead,BufReader,Read,Write},net::{Ipv4Addr,Shutdown,SocketAddrV4,TcpListener,TcpStream},path::{Path,PathBuf},sync::{Arc,Mutex,atomic::{AtomicBool,AtomicUsize,Ordering}},thread,time::Duration};

const MAX_REQUEST:usize=1024*1024;
const MAX_CLIENTS:usize=32;
const MAX_IN_FLIGHT:usize=16;

fn dir()->Result<PathBuf,String> {
    Ok(PathBuf::from(env::var_os("LOCALAPPDATA").ok_or("Нет LOCALAPPDATA")?).join("Screenmate").join("mcp"))
}
fn metadata_path()->Result<PathBuf,String>{Ok(dir()?.join("daemon.json"))}

/// Even a malicious or broken peer cannot grow an unbounded line allocation.
pub(crate) fn bounded_line<R:BufRead>(reader:&mut R,limit:usize)->io::Result<Option<Vec<u8>>> {
    let mut line=Vec::new();
    let n=reader.take((limit+1) as u64).read_until(b'\n',&mut line)?;
    if n==0 {return Ok(None);}
    if n>limit || !line.ends_with(b"\n") {return Err(io::Error::new(io::ErrorKind::InvalidData,"MCP message exceeds limit or lacks newline"));}
    Ok(Some(line))
}

fn same_secret(a:&[u8],b:&[u8])->bool {
    if a.len()!=b.len(){return false;}
    let mut mismatch=0u8;for (&x,&y) in a.iter().zip(b){mismatch|=x^y;}mismatch==0
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{os::windows::{ffi::OsStrExt,process::CommandExt},process::Command};
    const CREATE_NO_WINDOW:u32=0x08000000;
    #[link(name="bcrypt")]
    extern "system" {fn BCryptGenRandom(algorithm:isize,buffer:*mut u8,length:u32,flags:u32)->i32;}
    #[link(name="kernel32")]
    extern "system" {fn CreateMutexW(attributes:*const std::ffi::c_void,initial:i32,name:*const u16)->isize;fn GetLastError()->u32;fn CloseHandle(handle:isize)->i32;fn MoveFileExW(from:*const u16,to:*const u16,flags:u32)->i32;fn LocalFree(memory:isize)->isize;}
    #[link(name="advapi32")]
    extern "system" {
        fn ConvertStringSecurityDescriptorToSecurityDescriptorW(text:*const u16,revision:u32,descriptor:*mut *mut std::ffi::c_void,size:*mut u32)->i32;
        fn GetSecurityDescriptorDacl(descriptor:*const std::ffi::c_void,present:*mut i32,dacl:*mut *mut std::ffi::c_void,defaulted:*mut i32)->i32;
        fn SetNamedSecurityInfoW(name:*mut u16,object_type:u32,information:u32,owner:*mut std::ffi::c_void,group:*mut std::ffi::c_void,dacl:*mut std::ffi::c_void,sacl:*mut std::ffi::c_void)->u32;
    }

    pub struct Instance(isize);
    impl Drop for Instance {fn drop(&mut self){unsafe{CloseHandle(self.0);}}}
    /// None means another daemon in this user's Windows session owns it.
    pub fn instance()->Result<Option<Instance>,String> {
        let mut hash=std::collections::hash_map::DefaultHasher::new();
        let base=env::var_os("LOCALAPPDATA").ok_or("Нет LOCALAPPDATA")?;
        let canonical=fs::canonicalize(&base).unwrap_or_else(|_|PathBuf::from(base));
        canonical.to_string_lossy().to_lowercase().hash(&mut hash);
        let name:Vec<u16>=format!("Local\\Screenmate.MCP.{:016x}\0",hash.finish()).encode_utf16().collect();
        let handle=unsafe{CreateMutexW(std::ptr::null(),0,name.as_ptr())};
        if handle==0{return Err("Не удалось создать блокировку MCP".into());}
        if unsafe{GetLastError()}==183 {unsafe{CloseHandle(handle);};return Ok(None);}
        Ok(Some(Instance(handle)))
    }
    pub fn token()->Result<String,String> {
        let mut bytes=[0u8;32];
        if unsafe{BCryptGenRandom(0,bytes.as_mut_ptr(),bytes.len() as u32,2)}<0 {return Err("Не удалось создать токен MCP".into());}
        Ok(bytes.iter().map(|b|format!("{b:02x}")).collect())
    }
    pub fn secure_dir(path:&Path)->Result<(),String> {
        fs::create_dir_all(path).map_err(|e|e.to_string())?;
        let who=Command::new("whoami").args(["/user","/fo","csv","/nh"]).creation_flags(CREATE_NO_WINDOW).output().map_err(|e|e.to_string())?;
        if !who.status.success(){return Err("Не удалось определить пользователя".into());}
        let line=String::from_utf8_lossy(&who.stdout);
        let sid=line.split(',').nth(1).unwrap_or("").trim().trim_matches('"');
        if !sid.starts_with("S-1-") || !sid.chars().all(|c|c.is_ascii_digit()||c=='S'||c=='-') {return Err("Не удалось определить SID пользователя".into());}
        // Replace the DACL in one Windows security operation. In particular,
        // never reset it to inherited rights while stale metadata may exist.
        let sddl:Vec<u16>=format!("D:P(A;OICI;FA;;;{sid})\0").encode_utf16().collect();
        let mut descriptor=std::ptr::null_mut();
        if unsafe{ConvertStringSecurityDescriptorToSecurityDescriptorW(sddl.as_ptr(),1,&mut descriptor,std::ptr::null_mut())}==0 {return Err("Не удалось подготовить ACL MCP".into());}
        let mut present=0;let mut defaulted=0;let mut dacl=std::ptr::null_mut();
        let valid=unsafe{GetSecurityDescriptorDacl(descriptor,&mut present,&mut dacl,&mut defaulted)}!=0 && present!=0 && !dacl.is_null();
        let mut name:Vec<u16>=path.as_os_str().encode_wide().chain(Some(0)).collect();
        // SE_FILE_OBJECT, DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION.
        let status=if valid {unsafe{SetNamedSecurityInfoW(name.as_mut_ptr(),1,0x80000004,std::ptr::null_mut(),std::ptr::null_mut(),dacl,std::ptr::null_mut())}}else{1};
        unsafe{LocalFree(descriptor as isize);}
        if status!=0 {return Err("Не удалось ограничить доступ к папке MCP".into());}Ok(())
    }
    pub fn replace(from:&Path,to:&Path)->Result<(),String> {
        let from:Vec<u16>=from.as_os_str().encode_wide().chain(Some(0)).collect();
        let to:Vec<u16>=to.as_os_str().encode_wide().chain(Some(0)).collect();
        if unsafe{MoveFileExW(from.as_ptr(),to.as_ptr(),0x1|0x8)}==0{return Err("Не удалось записать данные сеанса MCP".into());}Ok(())
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;
    pub struct Instance;
    pub fn instance()->Result<Option<Instance>,String>{Err("MCP daemon поддерживается только в Windows".into())}
    pub fn token()->Result<String,String>{Err("MCP daemon поддерживается только в Windows".into())}
    pub fn secure_dir(_: &Path)->Result<(),String>{Err("MCP daemon поддерживается только в Windows".into())}
    pub fn replace(_: &Path,_: &Path)->Result<(),String>{Err("MCP daemon поддерживается только в Windows".into())}
}

fn write_metadata(port:u16,token:&str)->Result<PathBuf,String> {
    let path=metadata_path()?;
    let folder=path.parent().ok_or("Нет папки MCP")?;
    // The directory is restricted before any new token reaches the filesystem.
    platform::secure_dir(folder)?;
    let temp=folder.join(format!("daemon-{}-{port}.tmp",std::process::id()));
    let result=(|| {
        let mut f=OpenOptions::new().write(true).create_new(true).open(&temp).map_err(|e|e.to_string())?;
        let body=serde_json::to_vec(&json!({"version":1,"port":port,"token":token,"pid":std::process::id()})).map_err(|e|e.to_string())?;
        f.write_all(&body).and_then(|_|f.sync_all()).map_err(|e|e.to_string())?;drop(f);
        platform::replace(&temp,&path)?;Ok(path.clone())
    })();
    if result.is_err(){let _=fs::remove_file(&temp);}result
}

struct MetadataGuard {path:PathBuf,token:String}
impl Drop for MetadataGuard {
    fn drop(&mut self) {
        if read_metadata(&self.path).is_some_and(|(_,secret)|same_secret(secret.as_bytes(),self.token.as_bytes())) {let _=fs::remove_file(&self.path);}
    }
}
fn read_metadata(path:&Path)->Option<(u16,String)> {
    let file=File::open(path).ok()?;let mut data=Vec::new();file.take(4097).read_to_end(&mut data).ok()?;
    if data.len()>4096{return None;}
    let value:Value=serde_json::from_slice(&data).ok()?;
    if value["version"]!=1{return None;}
    let port=value["port"].as_u64().and_then(|v|u16::try_from(v).ok()).filter(|p|*p!=0)?;
    let token=value["token"].as_str().filter(|s|s.len()==64&&s.bytes().all(|b|b.is_ascii_hexdigit()))?.to_owned();
    Some((port,token))
}

struct Reservation(Arc<AtomicUsize>);
impl Drop for Reservation {fn drop(&mut self){self.0.fetch_sub(1,Ordering::AcqRel);}}
fn reserve(counter:&Arc<AtomicUsize>,limit:usize)->Option<Reservation> {
    let mut current=counter.load(Ordering::Acquire);
    loop {
        if current>=limit{return None;}
        match counter.compare_exchange_weak(current,current+1,Ordering::AcqRel,Ordering::Acquire) {
            Ok(_)=>return Some(Reservation(counter.clone())),
            Err(observed)=>current=observed,
        }
    }
}

fn respond(writer:&Arc<Mutex<TcpStream>>,value:&Value)->io::Result<()> {
    let mut writer=writer.lock().map_err(|_|io::Error::new(io::ErrorKind::Other,"writer unavailable"))?;
    serde_json::to_writer(&mut *writer,value)?;writer.write_all(b"\n")?;writer.flush()
}

fn connection(mut stream:TcpStream,secret:Arc<String>,stop:Arc<AtomicBool>,_client:Reservation) {
    let _=stream.set_read_timeout(Some(Duration::from_secs(5)));
    let Ok(read_stream)=stream.try_clone() else{return;};let mut reader=BufReader::new(read_stream);
    let Some(auth)=bounded_line(&mut reader,1024).ok().flatten() else{return;};
    let Ok(auth)=serde_json::from_slice::<Value>(&auth) else{return;};
    let valid=auth.get("token").and_then(Value::as_str).is_some_and(|token|same_secret(token.as_bytes(),secret.as_bytes()));
    if !valid {let _=stream.write_all(b"{\"ok\":false}\n");return;}
    if stream.write_all(b"{\"ok\":true}\n").is_err(){return;}
    if auth.get("command").and_then(Value::as_str)==Some("shutdown") {stop.store(true,Ordering::Release);return;}
    // Windows socket clones can retain distinct timeout settings.
    let _=stream.set_read_timeout(None);
    let _=reader.get_ref().set_read_timeout(None);
    // Bound a client that authenticates but stops consuming MCP replies.
    let _=stream.set_write_timeout(Some(Duration::from_secs(15)));
    let writer=Arc::new(Mutex::new(stream));
    let active=Arc::new(AtomicUsize::new(0));
    loop {
        let line=match bounded_line(&mut reader,MAX_REQUEST) {Ok(Some(line))=>line,Ok(None)|Err(_)=>break};
        let request:Value=match serde_json::from_slice(&line) {Ok(v)=>v,Err(_)=>{let _=respond(&writer,&json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}));continue;}};
        if request.get("id").is_none(){continue;}
        let Some(reservation)=reserve(&active,MAX_IN_FLIGHT) else {
            let _=respond(&writer,&json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32000,"message":"Too many requests"}}));continue;
        };
        let writer=writer.clone();
        thread::spawn(move || {let _reservation=reservation;if let Some(reply)=super::reply(&request){let _=respond(&writer,&reply);}});
    }
}

pub fn run()->Result<(),String> {
    let Some(_instance)=platform::instance()? else{return Ok(());};
    let listener=TcpListener::bind((Ipv4Addr::LOCALHOST,0)).map_err(|e|e.to_string())?;
    let port=listener.local_addr().map_err(|e|e.to_string())?.port();
    let token=platform::token()?;
    let path=write_metadata(port,&token)?;
    let _metadata=MetadataGuard {path,token:token.clone()};
    let secret=Arc::new(token);let clients=Arc::new(AtomicUsize::new(0));let stop=Arc::new(AtomicBool::new(false));
    listener.set_nonblocking(true).map_err(|e|e.to_string())?;
    while !stop.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream,_))=>{
                // On Windows accepted sockets can inherit the listener's
                // non-blocking mode; connection workers require blocking I/O.
                if stream.set_nonblocking(false).is_err(){continue;}
                let Some(reservation)=reserve(&clients,MAX_CLIENTS) else {continue};
                let secret=secret.clone();let stop=stop.clone();thread::spawn(move ||connection(stream,secret,stop,reservation));
            }
            Err(error) if error.kind()==io::ErrorKind::WouldBlock=>thread::sleep(Duration::from_millis(50)),
            Err(error)=>return Err(error.to_string()),
        }
    }
    Ok(())
}

/// Prefer the autostarted daemon, but never trust a stale discovery file.
pub fn connect()->Option<TcpStream> {
    connect_with_command(None)
}
fn connect_with_command(command:Option<&str>)->Option<TcpStream> {
    let (port,token)=read_metadata(&metadata_path().ok()?)?;
    let addr=SocketAddrV4::new(Ipv4Addr::LOCALHOST,port);
    let mut stream=TcpStream::connect_timeout(&addr.into(),Duration::from_millis(350)).ok()?;
    stream.set_read_timeout(Some(Duration::from_millis(700))).ok()?;
    stream.set_write_timeout(Some(Duration::from_millis(700))).ok()?;
    let auth=match command {Some(command)=>json!({"token":token,"command":command}),None=>json!({"token":token})}.to_string();
    stream.write_all(auth.as_bytes()).ok()?;stream.write_all(b"\n").ok()?;
    // The daemon sends nothing beyond this acknowledgement until it receives
    // the first MCP request, so dropping this temporary BufReader loses nothing.
    let mut ack=BufReader::new(stream.try_clone().ok()?);
    let reply=bounded_line(&mut ack,1024).ok()??;
    let value:Value=serde_json::from_slice(&reply).ok()?;
    if value["ok"]!=true{return None;}
    stream.set_read_timeout(None).ok()?;stream.set_write_timeout(None).ok()?;
    Some(stream)
}

/// Explicit, authenticated stop. A stale or inaccessible file is not reported
/// as success: it cannot prove whether a separate daemon is still running.
pub fn stop()->Result<(),String> {
    let path=metadata_path()?;
    match fs::metadata(&path) {
        Err(error) if error.kind()==io::ErrorKind::NotFound=>return Ok(()),
        Err(_)=>return Err("Не удалось прочитать состояние MCP".into()),
        Ok(_)=>{}
    }
    let stream=connect_with_command(Some("shutdown")).ok_or("Не удалось связаться с daemon MCP; данные сеанса могут быть устаревшими")?;
    drop(stream);Ok(())
}

/// Forward a Codex stdio stream without printing the handshake or secret.
pub fn bridge(mut stream:TcpStream)->io::Result<()> {
    let mut incoming=stream.try_clone()?;
    let input_finished=Arc::new(AtomicBool::new(false));
    let reader_done=input_finished.clone();
    let output=thread::spawn(move || -> io::Result<()> {
        let stdout=io::stdout();let mut out=stdout.lock();let mut buffer=[0u8;16384];
        loop {match incoming.read(&mut buffer) {
            Ok(0)=>{if !reader_done.load(Ordering::Acquire){std::process::exit(1);}return Ok(());},
            Err(error)=>{if !reader_done.load(Ordering::Acquire){std::process::exit(1);}return Err(error);},
            Ok(n)=>{if let Err(error)=out.write_all(&buffer[..n]).and_then(|_|out.flush()) {
                if !reader_done.load(Ordering::Acquire){std::process::exit(1);}return Err(error);
            }}
        }}
    });
    let result=io::copy(&mut io::stdin().lock(),&mut stream).map(|_|());
    // Let the daemon finish outstanding requests after stdin EOF. It closes
    // the TCP response stream only when all in-flight replies have drained.
    input_finished.store(true,Ordering::Release);
    let _=stream.shutdown(Shutdown::Write);drop(stream);
    let drained=output.join().unwrap_or_else(|_|Err(io::Error::new(io::ErrorKind::Other,"response thread failed")));
    result.and(drained)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn bounded_input_and_token_comparison() {
        let mut reader=BufReader::new(&b"ok\ntoolong\n"[..]);
        assert_eq!(bounded_line(&mut reader,4).unwrap(),Some(b"ok\n".to_vec()));
        assert!(bounded_line(&mut reader,4).is_err());
        assert!(same_secret(b"secret",b"secret"));assert!(!same_secret(b"secret",b"secreT"));assert!(!same_secret(b"secret",b""));
    }
    #[test] fn reservations_are_bounded_and_released() {
        let count=Arc::new(AtomicUsize::new(0));let a=reserve(&count,1).unwrap();assert!(reserve(&count,1).is_none());drop(a);assert!(reserve(&count,1).is_some());
    }
}
