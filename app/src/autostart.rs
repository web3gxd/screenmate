//! User-scoped Windows Run registration. We only touch our own entries and
//! treat a conflicting value as an error. StartupApproved is intentionally not
//! modified; Windows may independently disable a registered startup item.
use std::{fs::{self,OpenOptions},io::{Read,Write},path::{Path,PathBuf},sync::{Mutex,OnceLock}};
use serde::{Deserialize,Serialize};
use super::i18n::Language;

const APP:&str="Screenmate";
const MCP:&str="ScreenmateMcp";
const RUN_KEY:&str="Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RECORD:&str="autostart-preference.json";

#[derive(Clone,Debug,Serialize,Deserialize,PartialEq,Eq)]
#[serde(deny_unknown_fields)]
struct Record {version:u8,enabled:bool,app:String,mcp:String}
#[derive(Clone)]
struct Commands {app:String,mcp:String,legacy_app:String,mcp_path:PathBuf}
impl Commands {
    fn from_exe(path:&Path)->Result<Self,String> {
        let app=path.to_str().ok_or("Некорректный путь приложения")?;
        let mcp_path=path.parent().ok_or("Не найдена папка приложения")?.join("screenmate-mcp.exe");
        let mcp=mcp_path.to_str().ok_or("Некорректный путь MCP")?;
        if app.contains('"')||mcp.contains('"') {return Err("Некорректный путь приложения".into());}
        Ok(Self {app:format!("\"{app}\" --autostart"),mcp:format!("\"{mcp}\" --daemon"),legacy_app:format!("\"{app}\""),mcp_path:mcp_path.clone()})
    }
    fn expected(&self,name:&str)->&str {if name==APP {&self.app}else{&self.mcp}}
    fn owned(&self,name:&str,value:&str,old:Option<&Record>)->bool {
        let same=|x:&str,y:&str|x.to_lowercase()==y.to_lowercase();
        same(value,self.expected(name)) || (name==APP && same(value,&self.legacy_app)) || old.is_some_and(|r|same(value,if name==APP {&r.app}else{&r.mcp}))
    }
    fn record(&self,enabled:bool)->Record {Record{version:1,enabled,app:self.app.clone(),mcp:self.mcp.clone()}}
}

trait Store {
    fn get(&mut self,name:&str)->Result<Option<String>,String>;
    fn put(&mut self,name:&str,value:Option<&str>)->Result<(),String>;
    fn load(&mut self)->Result<Option<Record>,String>;
    fn save(&mut self,record:&Record)->Result<(),String>;
}
fn conflict()->String {"Обнаружена чужая или изменённая запись автозапуска; она не изменена".into()}
fn incomplete()->String {"Записи автозапуска неполные; проверьте настройки Windows".into()}
fn current<S:Store>(store:&mut S,commands:&Commands,old:Option<&Record>)->Result<[Option<String>;2],String> {
    let values=[store.get(APP)?,store.get(MCP)?];
    for (name,value) in [(APP,&values[0]),(MCP,&values[1])] {
        if value.as_ref().is_some_and(|v|!commands.owned(name,v,old)){return Err(conflict());}
    }
    Ok(values)
}
/// Change both values and the marker as one logical operation. If any step
/// fails, restore only values that still match what we wrote.
fn apply<S:Store>(store:&mut S,commands:&Commands,old:Option<&Record>,enabled:bool)->Result<(),String> {
    let before=current(store,commands,old)?;
    let desired=[enabled.then_some(commands.app.as_str()),enabled.then_some(commands.mcp.as_str())];
    let names=[APP,MCP];let mut changed=Vec::new();
    let apply_result=(||{
        for i in 0..2 {
            if before[i].as_deref()==desired[i] {continue;}
            if store.get(names[i])?!=before[i] {return Err(conflict());}
            store.put(names[i],desired[i])?;changed.push(i);
        }
        store.save(&commands.record(enabled))
    })();
    if let Err(error)=apply_result {
        let mut rollback_failed=false;
        for &i in changed.iter().rev() {
            match store.get(names[i]) {
                Ok(current) if current.as_deref()==desired[i]=>if store.put(names[i],before[i].as_deref()).is_err(){rollback_failed=true;},
                _=>rollback_failed=true,
            }
        }
        return if rollback_failed {Err(format!("{error}; не удалось полностью отменить изменение автозапуска"))} else {Err(error)};
    }
    Ok(())
}
fn initialize_store<S:Store>(store:&mut S,commands:&Commands,mcp_exists:bool)->Result<(),String> {
    let old=store.load()?;
    let values=current(store,commands,old.as_ref())?;
    match old.as_ref().map(|r|r.enabled) {
        None=>{
            if !mcp_exists {return Err("Не найден screenmate-mcp.exe рядом с приложением".into());}
            apply(store,commands,None,true)
        }
        Some(false)=>Ok(()), // never undo a deliberate opt-out
        Some(true) if values[0].is_none() && values[1].is_none()=>store.save(&commands.record(false)),
        Some(true) if values[0].is_none() || values[1].is_none()=>Err(incomplete()),
        Some(true)=>{
            if !mcp_exists {return Err("Не найден screenmate-mcp.exe рядом с приложением".into());}
            // The executable may have moved: migrate only values proven to be ours.
            apply(store,commands,old.as_ref(),true)
        }
    }
}
fn status_store<S:Store>(store:&mut S,commands:&Commands)->Result<bool,String> {
    let old=store.load()?;let values=current(store,commands,old.as_ref())?;
    match (&values[0],&values[1]) {
        (None,None)=>Ok(false),
        (Some(app),Some(mcp)) if app.to_lowercase()==commands.app.to_lowercase() && mcp.to_lowercase()==commands.mcp.to_lowercase()=>Ok(true),
        _=>Err(incomplete()),
    }
}
fn set_store<S:Store>(store:&mut S,commands:&Commands,enabled:bool,mcp_exists:bool)->Result<(),String> {
    if enabled && !mcp_exists {return Err("Не найден screenmate-mcp.exe рядом с приложением".into());}
    let old=store.load()?;apply(store,commands,old.as_ref(),enabled)
}

struct WindowsStore {record:PathBuf}
impl WindowsStore {fn new()->Result<Self,String>{Ok(Self{record:super::dir()?.join(RECORD)})}}
fn wide(s:&str)->Vec<u16>{s.encode_utf16().chain(Some(0)).collect()}
struct Key(windows_sys::Win32::System::Registry::HKEY);
impl Drop for Key {fn drop(&mut self){unsafe{windows_sys::Win32::System::Registry::RegCloseKey(self.0);}}}
fn open_key(write:bool,create:bool)->Result<Option<Key>,String> {
    use windows_sys::Win32::System::Registry::*;
    let mut key=0;let path=wide(RUN_KEY);
    let access=KEY_QUERY_VALUE | if write {KEY_SET_VALUE} else {0};
    let code=unsafe {if create {RegCreateKeyExW(HKEY_CURRENT_USER,path.as_ptr(),0,std::ptr::null(),0,access,std::ptr::null(),&mut key,std::ptr::null_mut())}
        else {RegOpenKeyExW(HKEY_CURRENT_USER,path.as_ptr(),0,access,&mut key)}};
    if code==2 && !create {return Ok(None);} if code!=0 {return Err(format!("Ошибка реестра Windows: {}",std::io::Error::from_raw_os_error(code as i32)));}
    Ok(Some(Key(key)))
}
impl Store for WindowsStore {
    fn get(&mut self,name:&str)->Result<Option<String>,String> {
        use windows_sys::Win32::System::Registry::*;
        let Some(key)=open_key(false,false)? else{return Ok(None)};let name=wide(name);let mut kind=0;let mut size=0u32;
        let code=unsafe{RegQueryValueExW(key.0,name.as_ptr(),std::ptr::null(),&mut kind,std::ptr::null_mut(),&mut size)};
        if code==2 {return Ok(None);} if code!=0 {return Err(format!("Ошибка чтения автозапуска: {}",std::io::Error::from_raw_os_error(code as i32)));}
        if kind!=REG_SZ || size>65536 || size%2!=0 {return Err(conflict());}
        let mut data=vec![0u16;(size as usize)/2];
        let code=unsafe{RegQueryValueExW(key.0,name.as_ptr(),std::ptr::null(),&mut kind,data.as_mut_ptr().cast(),&mut size)};
        if code!=0 {return Err(format!("Ошибка чтения автозапуска: {}",std::io::Error::from_raw_os_error(code as i32)));}
        if kind!=REG_SZ || size%2!=0 || size as usize>data.len()*2 {return Err(conflict());}
        data.truncate((size as usize)/2);if data.last()==Some(&0){data.pop();}
        if data.contains(&0){return Err(conflict());}
        Ok(Some(String::from_utf16(&data).map_err(|_|conflict())?))
    }
    fn put(&mut self,name:&str,value:Option<&str>)->Result<(),String> {
        use windows_sys::Win32::System::Registry::*;
        let key=open_key(true,value.is_some())?;let Some(key)=key else{return Ok(())};let name=wide(name);
        let code=if let Some(value)=value {let data=wide(value);unsafe{RegSetValueExW(key.0,name.as_ptr(),0,REG_SZ,data.as_ptr().cast(),(data.len()*2) as u32)}}
        else {unsafe{RegDeleteValueW(key.0,name.as_ptr())}};
        if code==0 || (code==2&&value.is_none()){Ok(())}else{Err(format!("Ошибка записи автозапуска: {}",std::io::Error::from_raw_os_error(code as i32)))}
    }
    fn load(&mut self)->Result<Option<Record>,String> {
        let file=match fs::File::open(&self.record) {Ok(f)=>f,Err(e) if e.kind()==std::io::ErrorKind::NotFound=>return Ok(None),Err(e)=>return Err(format!("Не удалось прочитать выбор автозапуска: {e}"))};
        let mut bytes=Vec::new();file.take(65537).read_to_end(&mut bytes).map_err(|e|e.to_string())?;
        if bytes.len()>65536{return Err("Настройки автозапуска слишком велики".into());}
        let record:Record=serde_json::from_slice(&bytes).map_err(|_|"Некорректные настройки автозапуска")?;
        if record.version!=1 {return Err("Неизвестная версия настроек автозапуска".into());}Ok(Some(record))
    }
    fn save(&mut self,record:&Record)->Result<(),String> {
        use std::os::windows::ffi::OsStrExt;
        #[link(name="kernel32")] extern "system" {fn MoveFileExW(a:*const u16,b:*const u16,flags:u32)->i32;}
        let parent=self.record.parent().ok_or("Нет папки настроек")?;
        let temp=parent.join(format!(".autostart-{}.tmp",uuid::Uuid::new_v4().simple()));
        let result=(||{
            let bytes=serde_json::to_vec(record).map_err(|e|e.to_string())?;
            let mut file=OpenOptions::new().write(true).create_new(true).open(&temp).map_err(|e|e.to_string())?;
            file.write_all(&bytes).and_then(|_|file.sync_all()).map_err(|e|e.to_string())?;drop(file);
            let a:Vec<u16>=temp.as_os_str().encode_wide().chain(Some(0)).collect();let b:Vec<u16>=self.record.as_os_str().encode_wide().chain(Some(0)).collect();
            if unsafe{MoveFileExW(a.as_ptr(),b.as_ptr(),0x1|0x8)}==0{return Err(std::io::Error::last_os_error().to_string());}Ok(())
        })();if result.is_err(){let _=fs::remove_file(&temp);}result
    }
}

fn lock()->std::sync::MutexGuard<'static,()> {static LOCK:OnceLock<Mutex<()>>=OnceLock::new();LOCK.get_or_init(||Mutex::new(())).lock().unwrap_or_else(|e|e.into_inner())}
fn localized(error:String,language:Language)->String {
    if language==Language::Ru {return error;}
    let translations=[
        ("Обнаружена чужая или изменённая запись автозапуска; она не изменена","Another application owns or changed an autostart entry; it was not modified"),
        ("Записи автозапуска неполные; проверьте настройки Windows","Autostart entries are incomplete; check Windows settings"),
        ("Не найден screenmate-mcp.exe рядом с приложением","screenmate-mcp.exe was not found next to the app"),
        ("не удалось полностью отменить изменение автозапуска","could not completely roll back the autostart change"),
        ("Ошибка реестра Windows","Windows registry error"),
        ("Ошибка чтения автозапуска","Could not read autostart"),
        ("Ошибка записи автозапуска","Could not write autostart"),
        ("Не удалось прочитать выбор автозапуска","Could not read the autostart preference"),
        ("Настройки автозапуска слишком велики","Autostart preferences are too large"),
        ("Некорректные настройки автозапуска","Invalid autostart preferences"),
        ("Неизвестная версия настроек автозапуска","Unknown autostart preference version"),
        ("Некорректный путь приложения","Invalid application path"),
        ("Некорректный путь MCP","Invalid MCP path"),
        ("Не найдена папка приложения","Could not find the app folder"),
        ("Нет папки настроек","Could not find the settings folder"),
    ];
    translations.into_iter().fold(error,|text,(from,to)|text.replace(from,to))
}
fn with_store<T>(language:Language,f:impl FnOnce(&mut WindowsStore,&Commands)->Result<T,String>)->Result<T,String> {
    let _guard=lock();
    (||{let mut store=WindowsStore::new()?;let exe=std::env::current_exe().map_err(|e|e.to_string())?;let commands=Commands::from_exe(&exe)?;f(&mut store,&commands)})().map_err(|e|localized(e,language))
}
pub fn initialize(language:Language)->Result<(),String> {with_store(language,|store,commands|initialize_store(store,commands,commands.mcp_path.is_file()))}
pub fn status(language:Language)->Result<bool,String> {with_store(language,|store,commands|{let enabled=status_store(store,commands)?;if enabled && !commands.mcp_path.is_file(){return Err("Не найден screenmate-mcp.exe рядом с приложением".into());}Ok(enabled)})}
pub fn set(enabled:bool,language:Language)->Result<(),String> {with_store(language,|store,commands|set_store(store,commands,enabled,commands.mcp_path.is_file()))}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    #[derive(Default)]
    struct Fake {values:BTreeMap<String,String>,record:Option<Record>,puts:usize,fail_put:Option<usize>,fail_save:bool}
    impl Store for Fake {
        fn get(&mut self,name:&str)->Result<Option<String>,String>{Ok(self.values.get(name).cloned())}
        fn put(&mut self,name:&str,value:Option<&str>)->Result<(),String>{
            self.puts+=1;if self.fail_put==Some(self.puts){return Err("access denied".into());}
            if let Some(value)=value{self.values.insert(name.into(),value.into());}else{self.values.remove(name);}Ok(())
        }
        fn load(&mut self)->Result<Option<Record>,String>{Ok(self.record.clone())}
        fn save(&mut self,record:&Record)->Result<(),String>{if self.fail_save{return Err("write denied".into());}self.record=Some(record.clone());Ok(())}
    }
    fn commands(path:&str)->Commands {Commands::from_exe(Path::new(path)).unwrap()}
    #[test] fn powershell_marker_json_is_serde_compatible() {
        // ConvertTo-Json emits UTF-8 without BOM and escapes the Windows paths.
        let json=br#"{"version":1,"enabled":true,"app":"\"C:\\Program Files\\Screenmate\\screenmate.exe\" --autostart","mcp":"\"C:\\Program Files\\Screenmate\\screenmate-mcp.exe\" --daemon"}"#;
        let record:Record=serde_json::from_slice(json).unwrap();
        assert!(record.enabled);assert!(record.app.ends_with("\" --autostart"));assert!(record.mcp.ends_with("\" --daemon"));
    }
    #[test] fn first_run_enables_both_and_explicit_optout_survives_restart() {
        let c=commands(r"C:\Program Files\Screenmate\screenmate.exe");let mut store=Fake::default();
        initialize_store(&mut store,&c,true).unwrap();assert_eq!(store.values.get(APP),Some(&c.app));assert_eq!(store.values.get(MCP),Some(&c.mcp));assert_eq!(status_store(&mut store,&c),Ok(true));
        set_store(&mut store,&c,false,true).unwrap();assert!(store.values.is_empty());assert!(!store.record.as_ref().unwrap().enabled);
        let writes=store.puts;initialize_store(&mut store,&c,true).unwrap();assert_eq!(writes,store.puts);assert_eq!(status_store(&mut store,&c),Ok(false));
        set_store(&mut store,&c,true,true).unwrap();assert_eq!(status_store(&mut store,&c),Ok(true));
    }
    #[test] fn legacy_and_moved_binary_are_migrated_only_when_proven_owned() {
        let old=commands(r"C:\Old Folder\screenmate.exe");let new=commands(r"D:\New Folder\renamed.exe");let mut store=Fake::default();
        store.values.insert(APP.into(),old.legacy_app.clone());initialize_store(&mut store,&old,true).unwrap();
        assert_eq!(store.values.get(APP),Some(&old.app));assert_eq!(store.values.get(MCP),Some(&old.mcp));
        initialize_store(&mut store,&new,true).unwrap();assert_eq!(store.values.get(APP),Some(&new.app));assert_eq!(store.values.get(MCP),Some(&new.mcp));
        assert!(new.app.starts_with('"')&&new.app.ends_with("\" --autostart"));assert!(new.mcp.ends_with("\" --daemon"));
    }
    #[test] fn foreign_or_incomplete_entries_are_never_overwritten() {
        let c=commands(r"C:\Screenmate\screenmate.exe");let mut store=Fake::default();store.values.insert(APP.into(),"other.exe".into());
        assert!(initialize_store(&mut store,&c,true).is_err());assert_eq!(store.values.get(APP).unwrap(),"other.exe");assert_eq!(store.puts,0);
        assert!(set_store(&mut store,&c,false,true).is_err());assert_eq!(store.puts,0);
        store.values.clear();store.values.insert(APP.into(),c.app.clone());store.record=Some(c.record(true));
        assert!(initialize_store(&mut store,&c,true).is_err());assert!(status_store(&mut store,&c).is_err());assert_eq!(store.puts,0);
        // An explicit user action may safely turn off our remaining entry.
        set_store(&mut store,&c,false,true).unwrap();assert!(store.values.is_empty());
    }
    #[test] fn missing_companion_and_partial_failures_roll_back_without_losing_prior_state() {
        let c=commands(r"C:\Screenmate\screenmate.exe");let mut store=Fake::default();
        assert!(initialize_store(&mut store,&c,false).is_err());assert!(store.values.is_empty());
        store.fail_put=Some(2);assert!(initialize_store(&mut store,&c,true).is_err());assert!(store.values.is_empty());assert!(store.record.is_none());
        let mut store=Fake::default();store.fail_save=true;assert!(initialize_store(&mut store,&c,true).is_err());assert!(store.values.is_empty());assert!(store.record.is_none());
    }
    #[test] fn external_removal_is_not_silently_reenabled() {
        let c=commands(r"C:\Screenmate\screenmate.exe");let mut store=Fake::default();initialize_store(&mut store,&c,true).unwrap();
        store.values.clear();let before=store.puts;initialize_store(&mut store,&c,true).unwrap();assert_eq!(store.puts,before);assert!(!store.record.unwrap().enabled);
    }
}
