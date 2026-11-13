//! User-managed application masking. Base config.yaml rules remain separate
//! and always take precedence over this optional, exact-executable list.
use std::{collections::BTreeSet, fs::{self,OpenOptions}, io::{Read,Write}, path::Path};
use serde::{Deserialize,Serialize};
use super::i18n::Language;

const MAX_FILE:u64=1024*1024;
#[derive(Serialize,Deserialize)]
#[serde(deny_unknown_fields)]
struct MaskFile {
    blocked_processes: Vec<String>,
    #[serde(default)]
    visible_only: bool,
}

#[derive(Clone,Debug)]
pub struct PrivacyApp {pub executable:String,pub configured:bool,pub selected:bool,pub running:bool}

fn executable_name(value:&str)->Result<String,String> {
    let name=value.trim();
    if name!=value || name.encode_utf16().count()>255 || name.chars().any(|c|c.is_control()||matches!(c,'\\'|'/'|':'|'*'|'?'|'"'|'<'|'>'|'|')) {
        return Err("Укажите только имя исполняемого файла .exe".into());
    }
    let normalized=name.to_lowercase();
    if normalized.len()<=4 || !normalized.ends_with(".exe") {return Err("Выберите приложение с расширением .exe".into());}
    Ok(normalized)
}
fn executable_name_for(value:&str,language:Language)->Result<String,String> {
    executable_name(value).map_err(|e| match e.as_str() {
        "Укажите только имя исполняемого файла .exe"=>language.text("Укажите только имя исполняемого файла .exe","Enter only an .exe file name").into(),
        _=>language.text("Выберите приложение с расширением .exe","Choose an application ending in .exe").into(),
    })
}
fn base_matches(config:&super::Config,exe:&str)->bool {
    config.blocked_processes.iter().any(|rule|!rule.is_empty()&&exe.contains(&rule.to_lowercase()))
}
fn screenmate_executable(exe: &str) -> bool {
    exe.eq_ignore_ascii_case("screenmate.exe")
}

// This is a presentation filter only: it never participates in capture
// authorization or in matching protected windows. Explicit rules are always
// retained, including when they name one of these service processes.
fn service_candidate(exe:&str)->bool {
    matches!(exe,
        "svchost.exe"|"conhost.exe"|"dllhost.exe"|"runtimebroker.exe"|
        "textinputhost.exe"|"sihost.exe"|"ctfmon.exe"|"searchhost.exe"|
        "searchapp.exe"|"shellexperiencehost.exe"|"startmenuexperiencehost.exe"|
        "nvidia overlay.exe"|"nvidia share.exe"|"nvcontainer.exe")
}
fn apps_from_running(config:&super::Config,language:Language,running:BTreeSet<String>)->Result<Vec<PrivacyApp>,String> {
    let mut names:BTreeSet<String>=running.iter().filter(|name|!service_candidate(name)||base_matches(config,name)).cloned().collect();
    for exe in &config.user_blocked_processes {names.insert(executable_name_for(exe,language)?);}
    // Base rules may be substrings rather than full .exe names. Keep them
    // visible as immutable rules even if no matching process is running.
    for rule in &config.blocked_processes {if !rule.is_empty(){names.insert(rule.to_lowercase());}}
    Ok(names.into_iter().filter(|exe| !screenmate_executable(exe)).map(|executable| {
        let configured=base_matches(config,&executable);
        let selected=configured||config.user_blocked_processes.iter().any(|item|item.to_lowercase()==executable);
        let running=running.contains(&executable);
        PrivacyApp {executable,configured,selected,running}
    }).collect())
}

/// Enumerate visible apps by executable only; never return window titles or
/// filesystem paths. Include explicitly saved apps even when they are closed.
pub fn apps(config:&super::Config,language:Language)->Result<Vec<PrivacyApp>,String> {
    let mut running=BTreeSet::new();
    for w in super::windows().map_err(|_|language.text("Не удалось получить список приложений","Could not list applications").to_owned())? {
        if !w.own {
            if let Ok(name)=executable_name(&w.process){running.insert(name);}
        }
    }
    apps_from_running(config,language,running)
}

#[cfg(test)]
fn load_user_masks(path:&Path)->Result<Vec<String>,String> {
    load_preferences(path,Language::Ru).map(|(names, _)| names)
}
pub fn load_preferences(path:&Path,language:Language)->Result<(Vec<String>, bool),String> {
    let read_error=language.text("Не удалось прочитать список защищённых приложений","Could not read the protected applications list");
    let file=match fs::File::open(path) {Ok(f)=>f,Err(e) if e.kind()==std::io::ErrorKind::NotFound=>return Ok((Vec::new(), false)),Err(e)=>return Err(format!("{read_error}: {e}"))};
    let mut bytes=Vec::new();file.take(MAX_FILE+1).read_to_end(&mut bytes).map_err(|e|format!("{read_error}: {e}"))?;
    if bytes.len() as u64>MAX_FILE {return Err(language.text("Список защищённых приложений слишком большой","The protected applications list is too large").into());}
    let saved:MaskFile=serde_yaml::from_slice(&bytes).map_err(|e|format!("{}: {e}",language.text("Ошибка списка защищённых приложений","Invalid protected applications list")))?;
    let mut names=BTreeSet::new();for name in saved.blocked_processes {names.insert(executable_name_for(&name,language)?);}
    Ok((names.into_iter().collect(), saved.visible_only))
}

#[cfg(windows)]
fn replace_file(source:&Path,destination:&Path,language:Language)->Result<(),String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name="kernel32")] extern "system" {fn MoveFileExW(existing:*const u16,new:*const u16,flags:u32)->i32;}
    let src:Vec<u16>=source.as_os_str().encode_wide().chain(Some(0)).collect();
    let dst:Vec<u16>=destination.as_os_str().encode_wide().chain(Some(0)).collect();
    if unsafe{MoveFileExW(src.as_ptr(),dst.as_ptr(),0x1|0x8)}==0 {return Err(format!("{}: {}",language.text("Не удалось сохранить список защищённых приложений","Could not save the protected applications list"),std::io::Error::last_os_error()));}
    Ok(())
}
#[cfg(not(windows))]
fn replace_file(source:&Path,destination:&Path,_language:Language)->Result<(),String> {fs::rename(source,destination).map_err(|e|e.to_string())}

fn save_preferences(path:&Path,names:&[String],visible_only:bool,language:Language)->Result<(),String> {
    let parent=path.parent().ok_or(language.text("Не найдена папка настроек","Settings folder not found"))?;
    let temp=parent.join(format!(".blocked-apps-{}.tmp",uuid::Uuid::new_v4().simple()));
    let result=(|| {
        let content=serde_yaml::to_string(&MaskFile {blocked_processes:names.to_vec(), visible_only}).map_err(|e|e.to_string())?;
        if content.len() as u64>MAX_FILE {return Err(language.text("Список защищённых приложений слишком большой","The protected applications list is too large").into());}
        let save_error=language.text("Не удалось сохранить список защищённых приложений","Could not save the protected applications list");
        let mut file=OpenOptions::new().write(true).create_new(true).open(&temp).map_err(|e|format!("{save_error}: {e}"))?;
        file.write_all(content.as_bytes()).and_then(|_|file.sync_all()).map_err(|e|format!("{save_error}: {e}"))?;
        drop(file);replace_file(&temp,path,language)
    })();
    if result.is_err(){let _=fs::remove_file(&temp);}
    result
}

/// The caller holds the shared mutex, serializing capture and settings changes.
/// Persist first; only after a successful atomic replacement update memory.
pub fn set_user_mask(state:&mut super::State,process:&str,checked:bool)->Result<(),String> {
    let path=super::dir().map_err(|_|state.language.text("Не найдена папка настроек","Settings folder not found").to_owned())?.join("blocked-apps.yaml");
    set_user_mask_at(state,&path,process,checked)
}
pub fn set_visible_only(state: &mut super::State, visible_only: bool) -> Result<(), String> {
    let path = super::dir().map_err(|_|state.language.text("Не найдена папка настроек","Settings folder not found").to_owned())?.join("blocked-apps.yaml");
    set_visible_only_at(state, &path, visible_only)
}
fn set_visible_only_at(state: &mut super::State, path: &Path, visible_only: bool) -> Result<(), String> {
    let (mut names, _) = load_preferences(path, state.language)?;
    names.extend(state.config.user_blocked_processes.iter().cloned());
    names.sort_unstable();
    names.dedup();
    save_preferences(path, &names, visible_only, state.language)?;
    state.config.user_blocked_processes = names;
    state.config.visible_only = visible_only;
    state.frames.clear();
    state.last_look = None;
    Ok(())
}
fn set_user_mask_at(state:&mut super::State,path:&Path,process:&str,checked:bool)->Result<(),String> {
    let language=state.language;
    let name=executable_name_for(process,language)?;
    if checked && screenmate_executable(&name) {
        return Err(language.text("Окно Скринмейта всегда остаётся видимым","ScreenMate's window is always visible").into());
    }
    if !checked && base_matches(&state.config,&name) {return Err(language.text("Это приложение защищено базовыми правилами config.yaml","This application is protected by the base config.yaml rules").into());}
    let (saved, _) = load_preferences(path, language)?;
    let mut names:BTreeSet<String>=saved.into_iter().collect();
    for previous in &state.config.user_blocked_processes {names.insert(executable_name_for(previous,language)?);}
    if checked {names.insert(name);} else {names.remove(&name);}
    let next:Vec<String>=names.into_iter().collect();
    save_preferences(path,&next,state.config.visible_only,language)?;
    state.config.user_blocked_processes=next;
    // Frames made under the previous policy must never linger in the ring.
    state.frames.clear();state.last_look=None;
    Ok(())
}

/// Show a native local executable picker on the GUI thread, outside State's
/// mutex. The returned value is an executable basename, never its full path.
pub fn choose_executable(language:Language)->Result<Option<String>,String> {
    #[repr(C)] struct OpenFileName {
        size:u32,owner:isize,instance:isize,filter:*const u16,custom_filter:*mut u16,max_custom_filter:u32,
        filter_index:u32,file:*mut u16,max_file:u32,file_title:*mut u16,max_file_title:u32,
        initial_dir:*const u16,title:*const u16,flags:u32,file_offset:u16,file_extension:u16,
        default_extension:*const u16,custom_data:isize,hook:usize,template:*const u16,
        reserved:*mut std::ffi::c_void,reserved_word:u32,flags_ex:u32,
    }
    #[link(name="comdlg32")] extern "system" {fn GetOpenFileNameW(ofn:*mut OpenFileName)->i32;fn CommDlgExtendedError()->u32;}
    #[link(name="user32")] extern "system" {fn GetActiveWindow()->isize;}
    let filter:Vec<u16>=format!("{}\0*.exe\0\0",language.text("Приложения (*.exe)","Applications (*.exe)")).encode_utf16().collect();
    let title:Vec<u16>=format!("{}\0",language.text("Выберите приложение для защиты","Choose an application to protect")).encode_utf16().collect();
    let mut file=vec![0u16;32768];
    let mut ofn:OpenFileName=unsafe{std::mem::zeroed()};
    ofn.size=std::mem::size_of::<OpenFileName>() as u32;ofn.owner=unsafe{GetActiveWindow()};ofn.filter=filter.as_ptr();ofn.filter_index=1;
    ofn.file=file.as_mut_ptr();ofn.max_file=file.len() as u32;ofn.title=title.as_ptr();
    ofn.flags=0x0008_0000|0x0000_1000|0x0000_0800|0x0000_0008|0x0000_0004;
    if unsafe{GetOpenFileNameW(&mut ofn)}==0 {
        let code=unsafe{CommDlgExtendedError()};return if code==0 {Ok(None)}else{Err(format!("{} ({code:#x})",language.text("Не удалось открыть выбор приложения","Could not open the application picker")))};
    }
    let end=file.iter().position(|&c|c==0).ok_or(language.text("Не удалось прочитать выбранное имя","Could not read the selected name"))?;
    let full=String::from_utf16(&file[..end]).map_err(|_|language.text("Недопустимое имя приложения","Invalid application name"))?;
    let name=full.rsplit(['\\','/']).next().ok_or(language.text("Не удалось прочитать выбранное имя","Could not read the selected name"))?;
    Ok(Some(executable_name_for(name,language)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    struct TestDir(PathBuf);
    impl TestDir {
        fn new()->Self {let path=std::env::temp_dir().join(format!("screenmate-masks-test-{}",uuid::Uuid::new_v4().simple()));fs::create_dir(&path).unwrap();Self(path)}
    }
    impl Drop for TestDir {fn drop(&mut self){let _=fs::remove_dir_all(&self.0);}}
    fn state()->super::super::State {
        super::super::State {
            config:super::super::Config {blocked_titles:vec!["банк".into()],blocked_processes:vec!["secret.exe".into()],user_blocked_processes:vec![],visible_only:false,ring_size:5,jpeg_quality:70,max_width:1920},
            paused:false,monitor:None,window:None,region:None,frames:std::collections::VecDeque::new(),last_look:Some(42),inbox:vec![],accent:[90,197,235],glow_enabled:true,glow_preference:true,minimize_to_tray:true,error:String::new(),language:Language::Ru,capture_activity:std::sync::Arc::new(std::sync::Mutex::new(crate::activity::CaptureActivity::default())),
        }
    }
    #[test] fn normalize_executables_and_reject_paths_or_patterns() {
        assert_eq!(executable_name("APP.EXE").unwrap(),"app.exe");
        assert_eq!(executable_name("ТЕСТ.exe").unwrap(),"тест.exe");
        for invalid in ["", ".exe", "a", "C:\\secret.exe", "../a.exe", "a*.exe", "a.exe ", "a\n.exe"] {assert!(executable_name(invalid).is_err(),"accepted {invalid:?}");}
    }
    #[test] fn service_filter_affects_suggestions_only_and_preserves_explicit_masks() {
        let mut s=state();
        let running:BTreeSet<_>=["svchost.exe","textinputhost.exe","nvidia overlay.exe","editor.exe","screenmate.exe"].into_iter().map(str::to_owned).collect();
        let list=apps_from_running(&s.config,Language::Ru,running.clone()).unwrap();
        assert!(list.iter().any(|a|a.executable=="editor.exe" && a.running));
        for name in ["svchost.exe","textinputhost.exe","nvidia overlay.exe","screenmate.exe"] {assert!(!list.iter().any(|a|a.executable==name));}
        s.config.blocked_processes.push("svchost".into());
        s.config.user_blocked_processes.push("nvidia overlay.exe".into());
        let list=apps_from_running(&s.config,Language::Ru,running).unwrap();
        assert!(list.iter().any(|a|a.executable=="svchost" && a.configured));
        assert!(list.iter().any(|a|a.executable=="svchost.exe" && a.configured && a.running));
        assert!(list.iter().any(|a|a.executable=="nvidia overlay.exe" && a.selected && a.running));
        let protected=super::super::Window {id:1,title:String::new(),process:"NVIDIA Overlay.EXE".into(),x:0,y:0,width:2,height:2,own:false,occlusion_rect:None};
        assert!(super::super::blocked(&protected,&s.config),"a filtered process must still be masked");
    }
    #[test] fn save_reload_add_remove_preserves_base_rules_and_other_entries() {
        let dir=TestDir::new();let file=dir.0.join("blocked-apps.yaml");let config=dir.0.join("config.yaml");
        let original=b"blocked_titles: [bank]\nblocked_processes: [secret.exe]\n# keep this comment\n";fs::write(&config,original).unwrap();
        fs::write(&file,b"blocked_processes: [other.exe]\n").unwrap();
        assert!(!load_preferences(&file, Language::Ru).unwrap().1);
        let mut s=state();s.config.user_blocked_processes.push("keep.exe".into());
        set_user_mask_at(&mut s,&file,"APP.EXE",true).unwrap();
        assert_eq!(load_user_masks(&file).unwrap(),vec!["app.exe","keep.exe","other.exe"]);
        assert_eq!(s.config.user_blocked_processes,load_user_masks(&file).unwrap());
        assert_eq!(fs::read(&config).unwrap(),original);assert_eq!(s.config.blocked_titles,vec!["банк"]);assert_eq!(s.config.blocked_processes,vec!["secret.exe"]);
        assert!(s.last_look.is_none());
        set_user_mask_at(&mut s,&file,"app.exe",false).unwrap();
        assert_eq!(load_user_masks(&file).unwrap(),vec!["keep.exe","other.exe"]);
        assert!(set_user_mask_at(&mut s,&file,"secret.exe",false).is_err());
        s.last_look = Some(42);
        set_visible_only_at(&mut s, &file, true).unwrap();
        assert!(s.config.visible_only);
        assert!(s.last_look.is_none());
        assert!(set_user_mask_at(&mut s,&file,"ScreenMate.exe",true).is_err());
        set_user_mask_at(&mut s, &file, "new.exe", true).unwrap();
        assert_eq!(load_preferences(&file, Language::Ru).unwrap(), (vec!["keep.exe".into(), "new.exe".into(), "other.exe".into()], true));
    }
    #[test] fn failed_save_and_corrupt_file_do_not_change_runtime_protection() {
        let dir=TestDir::new();let mut s=state();s.config.user_blocked_processes=vec!["keep.exe".into()];
        let missing=dir.0.join("missing").join("blocked-apps.yaml");
        assert!(set_visible_only_at(&mut s, &missing, true).is_err());
        assert!(!s.config.visible_only);
        assert!(set_user_mask_at(&mut s,&missing,"new.exe",true).is_err());
        assert_eq!(s.config.user_blocked_processes,vec!["keep.exe"]);assert_eq!(s.last_look,Some(42));
        let corrupt=dir.0.join("blocked-apps.yaml");fs::write(&corrupt,b"invalid: [").unwrap();
        assert!(load_user_masks(&corrupt).is_err());
        assert!(set_user_mask_at(&mut s,&corrupt,"keep.exe",false).is_err());
        assert_eq!(s.config.user_blocked_processes,vec!["keep.exe"]);
    }
    #[test] fn validation_errors_follow_language() {
        assert_eq!(executable_name_for("dir/app.exe",Language::En).unwrap_err(),"Enter only an .exe file name");
        assert_eq!(executable_name_for("app",Language::En).unwrap_err(),"Choose an application ending in .exe");
        assert_eq!(executable_name_for("app",Language::Ru).unwrap_err(),"Выберите приложение с расширением .exe");
    }
}
