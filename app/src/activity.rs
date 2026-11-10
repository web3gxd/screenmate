//! Short-lived indication of an explicit capture request. The background ring
//! buffer never touches this state.
use std::time::{Duration,Instant};

pub const GLOW_HOLD:Duration=Duration::from_millis(2400);

/// Only the explicit Run-entry marker denotes an autostart session; ordinary
/// launches must preserve the user's saved preference.
pub fn has_autostart_arg(args:impl IntoIterator<Item=std::ffi::OsString>)->bool {
    args.into_iter().any(|arg|arg==std::ffi::OsStr::new("--autostart"))
}
pub fn startup_glow_permission(saved:bool,autostart:bool)->bool {saved||autostart}

#[derive(Default)]
pub struct CaptureActivity {last:Option<(Instant,Option<u32>)>}
impl CaptureActivity {
    pub fn record(&mut self,now:Instant,monitor:Option<u32>){self.last=Some((now,monitor));}
    pub fn clear(&mut self){self.last=None;}
    /// Outer None means no active capture indication; inner None selects the
    /// primary monitor and Some(id) selects that exact monitor.
    pub fn monitor(&self,now:Instant)->Option<Option<u32>> {
        self.last.and_then(|(when,monitor)|now.checked_duration_since(when).filter(|age|*age<GLOW_HOLD).map(|_|monitor))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn activity_is_inactive_at_start_expires_and_uses_last_actual_monitor() {
        let start=Instant::now();let mut activity=CaptureActivity::default();
        assert_eq!(activity.monitor(start),None);
        activity.record(start,None);assert_eq!(activity.monitor(start),Some(None));
        activity.record(start+Duration::from_secs(1),Some(7));
        assert_eq!(activity.monitor(start+Duration::from_millis(3399)),Some(Some(7)));
        assert_eq!(activity.monitor(start+Duration::from_millis(3400)),None);
        activity.clear();assert_eq!(activity.monitor(start+Duration::from_secs(1)),None);
    }
    #[test] fn autostart_enables_only_session_permission_without_signalling_capture() {
        assert!(has_autostart_arg([std::ffi::OsString::from("--autostart")]));
        assert!(!has_autostart_arg([std::ffi::OsString::from("--autostart=true")]));
        assert!(!startup_glow_permission(false,false));
        assert!(startup_glow_permission(false,true));
        assert!(startup_glow_permission(true,false));
        // Permission alone, including on autostart, never displays a glow.
        let a=CaptureActivity::default();assert_eq!(a.monitor(Instant::now()),None);
    }
}
