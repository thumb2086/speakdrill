//! VoiceMeeter Remote API（透過 VoicemeeterRemote64.dll）
//!
//! 只包我們需要的參數操作。注意 DLL 裡沒有 `VBVMR_Run`，
//! 執行檔那支叫 `VBVMR_RunVoicemeeter`。

use anyhow::{anyhow, Result};
use std::ffi::CString;

#[cfg(windows)]
pub struct Vm {
    _lib: libloading::Library,
    set_f: unsafe extern "system" fn(*const i8, f32) -> i32,
    get_f: unsafe extern "system" fn(*const i8, *mut f32) -> i32,
    typ_f: unsafe extern "system" fn(*mut i32) -> i32,
    logout_f: unsafe extern "system" fn() -> i32,
    pub vm_type: i32,
}

#[cfg(windows)]
impl Vm {
    pub fn connect() -> Result<Self> {
        let candidates = [
            r"C:\Program Files (x86)\VB\Voicemeeter\VoicemeeterRemote64.dll",
            r"C:\Program Files\VB\Voicemeeter\VoicemeeterRemote64.dll",
            r"C:\Program Files (x86)\VB\Voicemeeter\VoicemeeterRemote.dll",
        ];
        let path = candidates
            .iter()
            .find(|p| std::path::Path::new(p).exists())
            .ok_or_else(|| anyhow!("找不到 VoiceMeeter Remote DLL，請先安裝 VoiceMeeter"))?;

        unsafe {
            let lib = libloading::Library::new(path)
                .map_err(|e| anyhow!("載入 {path} 失敗：{e}"))?;
            let login: libloading::Symbol<unsafe extern "system" fn() -> i32> = lib.get(b"VBVMR_Login")?;
            login();
            let typ_f = *lib.get::<unsafe extern "system" fn(*mut i32) -> i32>(b"VBVMR_GetVoicemeeterType")?;
            let set_f = *lib.get::<unsafe extern "system" fn(*const i8, f32) -> i32>(b"VBVMR_SetParameterFloat")?;
            let get_f = *lib.get::<unsafe extern "system" fn(*const i8, *mut f32) -> i32>(b"VBVMR_GetParameterFloat")?;
            let logout_f = *lib.get::<unsafe extern "system" fn() -> i32>(b"VBVMR_Logout")?;

            let mut t = 0i32;
            typ_f(&mut t);
            Ok(Self { _lib: lib, set_f, get_f, typ_f, logout_f, vm_type: t })
        }
    }

    pub fn kind(&self) -> &'static str {
        match self.vm_type {
            1 => "Standard",
            2 => "Banana",
            3 => "Potato",
            _ => "unknown",
        }
    }

    pub fn set(&self, name: &str, v: f32) -> Result<()> {
        let c = CString::new(name)?;
        let hr = unsafe { (self.set_f)(c.as_ptr() as *const i8, v) };
        if hr != 0 {
            return Err(anyhow!("SetParameterFloat({name}) hr={hr}"));
        }
        Ok(())
    }

    pub fn get(&self, name: &str) -> Result<f32> {
        let c = CString::new(name)?;
        let mut v = 0f32;
        let hr = unsafe { (self.get_f)(c.as_ptr() as *const i8, &mut v) };
        if hr != 0 {
            return Err(anyhow!("GetParameterFloat({name}) hr={hr}"));
        }
        Ok(v)
    }

    pub fn refresh_type(&mut self) {
        let mut t = 0i32;
        unsafe { (self.typ_f)(&mut t) };
        self.vm_type = t;
    }

    /// 預設 strip / bus 對應（實測：Banana 的 Voicemeeter Input = strip[3]，B1 = bus[3]）
    pub fn default_route(&self) -> (usize, usize) {
        match self.vm_type {
            3 => (5, 5), // Potato: VAIO=strip[5], B1=bus[5]
            _ => (3, 3), // Banana: VAIO=strip[3], B1=bus[3]
        }
    }

    /// 把指定 strip 送到 B1，其他 strip 不送
    pub fn route_to_b1(&self, strip: usize) -> Result<()> {
        let n = match self.vm_type { 1 => 3, 2 => 5, 3 => 8, _ => 5 };
        for j in 0..n {
            self.set(&format!("strip[{j}].B1"), if j == strip { 1.0 } else { 0.0 })?;
            self.set(&format!("strip[{j}].mute"), 0.0)?;
        }
        self.set(&format!("strip[{strip}].gain"), 0.0)?;
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for Vm {
    fn drop(&mut self) {
        unsafe { (self.logout_f)(); }
    }
}

#[cfg(not(windows))]
pub struct Vm;
#[cfg(not(windows))]
impl Vm {
    pub fn connect() -> Result<Self> {
        Err(anyhow!("VoiceMeeter 只支援 Windows"))
    }
}
