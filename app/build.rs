//! Embed the Windows executable icon without adding runtime dependencies.
//! If the SDK resource compiler is unavailable the program still builds;
//! Cargo reports a warning and the binary simply has no embedded icon.
use std::{env,fs,path::{Path,PathBuf},process::Command};

fn resource_compiler()->Option<PathBuf> {
    if let Some(rc)=env::var_os("RC") {return Some(PathBuf::from(rc));}
    if let (Some(root),Some(version))=(env::var_os("WindowsSdkDir"),env::var_os("WindowsSDKVersion")) {
        let path=PathBuf::from(root).join("bin").join(version).join("x64").join("rc.exe");
        if path.is_file(){return Some(path);}
    }
    if let Some(root)=env::var_os("ProgramFiles(x86)") {
        let sdk=PathBuf::from(root).join("Windows Kits").join("10").join("bin");
        if let Ok(entries)=fs::read_dir(sdk) {
            let mut candidates:Vec<_>=entries.filter_map(Result::ok).map(|e|e.path().join("x64").join("rc.exe")).filter(|p|p.is_file()).collect();
            candidates.sort();if let Some(path)=candidates.pop(){return Some(path);}
        }
    }
    // Command lookup is performed by Windows for a conventional SDK PATH.
    Some(PathBuf::from("rc.exe"))
}

fn embed(icon:&Path,out:&Path)->Result<(),String> {
    let script=out.join("screenmate.rc");let resource=out.join("screenmate.res");
    let icon=icon.to_string_lossy().replace('\\',"\\\\").replace('"',"\\\"");
    let contents=format!(r#"#pragma code_page(65001)
1 ICON "{icon}"
1 VERSIONINFO
 FILEVERSION 0,1,0,0
 PRODUCTVERSION 0,1,0,0
 FILEFLAGSMASK 0x3fL
 FILEOS 0x4L
 FILETYPE 0x1L
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "FileDescription", "Screenmate"
      VALUE "FileVersion", "0.1.0"
      VALUE "ProductName", "Screenmate"
      VALUE "ProductVersion", "0.1.0"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x0409, 1200
  END
END
"#);
    fs::write(&script,contents).map_err(|e|e.to_string())?;
    let rc=resource_compiler().ok_or("Windows SDK rc.exe not found")?;
    let result=Command::new(rc).arg("/nologo").arg("/fo").arg(&resource).arg(&script).output().map_err(|e|e.to_string())?;
    if !result.status.success() {return Err(String::from_utf8_lossy(&result.stderr).into_owned()+&String::from_utf8_lossy(&result.stdout));}
    println!("cargo:rustc-link-arg={}",resource.display());
    Ok(())
}
fn main() {
    println!("cargo:rerun-if-changed=assets/screenmate.ico");
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref()!=Ok("windows") {return;}
    if env::var("CARGO_CFG_TARGET_ENV").as_deref()!=Ok("msvc") || !cfg!(windows) {
        println!("cargo:warning=Screenmate icon embedding requires the Windows MSVC SDK; continuing without the icon.");return;
    }
    let icon=PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("assets").join("screenmate.ico");
    let out=PathBuf::from(env::var_os("OUT_DIR").unwrap());
    if let Err(error)=embed(&icon,&out) {println!("cargo:warning=Could not embed Screenmate icon: {error}");}
}
