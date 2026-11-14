//! 使用 Windows 隱藏的 per-app audio endpoint 內部介面。
//!
//! 這是 Windows Settings 實際用的機制，EarTrumpet/SoundSwitch 也在用它。
//! 不會寫入 registry，也不會修改全域預設裝置。

use anyhow::{anyhow, Result};
use windows::core::GUID;

const CLASS_ID: &str = "Windows.Media.Internal.AudioPolicyConfig";
// Windows 11 26H2 IID（本機已驗證）
const IID: GUID = GUID::from_u128(0xab3d4648_e242_459f_b02f_541c70306324);

#[cfg(windows)]
mod native {
    use super::GUID;
    use std::ffi::c_void;
    use std::sync::OnceLock;
    use windows::core::HRESULT;

    #[allow(dead_code)]
type RawFn = unsafe extern "system" fn(*mut c_void) -> HRESULT;
    type CreateString = unsafe extern "system" fn(*const u16, u32, *mut *mut c_void) -> HRESULT;
    type DeleteString = unsafe extern "system" fn(*mut c_void) -> HRESULT;
    type GetActivationFactory =
        unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct FactoryPtr(*mut c_void);

    struct Combase {
        create_string: CreateString,
        delete_string: DeleteString,
        activate: GetActivationFactory,
    }

    static COMBASE: OnceLock<Result<Combase, String>> = OnceLock::new();

    fn combase() -> Result<&'static Combase, String> {
        COMBASE
            .get_or_init(|| unsafe {
                let lib = libloading::Library::new("combase.dll")
                    .map_err(|e| format!("combase.dll: {e}"))?;
                let create: CreateString =
                    *lib.get::<CreateString>(b"WindowsCreateString")
                        .map_err(|e| format!("WindowsCreateString: {e}"))?;
                let delete: DeleteString =
                    *lib.get::<DeleteString>(b"WindowsDeleteString")
                        .map_err(|e| format!("WindowsDeleteString: {e}"))?;
                let activate: GetActivationFactory =
                    *lib.get::<GetActivationFactory>(b"RoGetActivationFactory")
                        .map_err(|e| format!("RoGetActivationFactory: {e}"))?;
                Ok(Combase {
                    create_string: create,
                    delete_string: delete,
                    activate,
                })
            })
            .as_ref()
            .map_err(|e| e.clone())
    }

    unsafe fn hstring(utf16: &[u16]) -> Result<*mut c_void, String> {
        let c = combase()?;
        let mut out = std::ptr::null_mut();
        let _ = (c.create_string)(utf16.as_ptr(), utf16.len() as u32, &mut out).ok();
        if out.is_null() {
            Err("WindowsCreateString failed".into())
        } else {
            Ok(out)
        }
    }

    fn utf16z(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    #[repr(transparent)]
    struct Factory(FactoryPtr);

    impl Factory {
        fn activate() -> Result<Self, String> {
            let c = combase()?;
            let class = utf16z(super::CLASS_ID);
            let h_class = unsafe { hstring(&class[..class.len() - 1])? };
            let mut raw = std::ptr::null_mut();
            let hr = unsafe { (c.activate)(h_class, &super::IID, &mut raw) };
            unsafe {
                let _ = (c.delete_string)(h_class);
            }
            hr.ok().map_err(|e| format!("activation failed: {e}"))?;
            if raw.is_null() {
                return Err("activation returned null".into());
            }
            Ok(Self(FactoryPtr(raw)))
        }

        unsafe fn slot(&self, index: usize) -> usize {
            let vtbl = *(self.0 .0 as *const *const usize);
            *vtbl.add(index)
        }

        fn set(&self, pid: u32, flow: i32, role: i32, device_id: &str) -> Result<(), String> {
            let token = utf16z(device_id);
            let h_id = unsafe { hstring(&token[..token.len() - 1])? };
            type Set = unsafe extern "system" fn(*mut c_void, u32, i32, i32, *mut c_void) -> HRESULT;
            let set: Set = unsafe { std::mem::transmute(self.slot(25)) };
            let hr = unsafe { set(self.0.0, pid, flow, role, h_id) };
            unsafe {
                let _ = (combase()?.delete_string)(h_id);
            }
            hr.ok().map_err(|e| format!("SetPersistedDefaultAudioEndpoint failed: {e}"))
        }
    }

    impl Drop for Factory {
        fn drop(&mut self) {
            unsafe {
                let release: RawFn = std::mem::transmute(self.slot(2));
                let _ = release(self.0 .0);
            }
        }
    }

    pub fn configure(path: &str, pid: u32, capture: bool) -> Result<(), String> {
        let raw_id = crate::winvol::find_endpoint_id_any(path)
            .ok_or_else(|| format!("cannot resolve MMDevice ID from {path}"))?;

        let mirror = if raw_id.contains("0.0.1") {
            "{2eef81be-33fa-4800-9670-1cd474972c3f}"
        } else {
            "{e6327cad-dcec-4949-ae8a-991e976a79d2}"
        };
        let token = format!("\\\\?\\SWD#MMDEVAPI#{raw_id}#{mirror}");

        let f = Factory::activate()?;
        let flow = if capture { 1 } else { 0 };
        for role in [0, 1, 2] { // eConsole, eMultimedia, eCommunications
            f.set(pid, flow, role, &token)?;
        }
        Ok(())
    }
}

pub fn configure(path: &str, pid: u32, capture: bool) -> Result<()> {
    #[cfg(windows)]
    {
        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            );
        }
        native::configure(path, pid, capture).map_err(|e| anyhow!(e))
    }
    #[cfg(not(windows))]
    Err(anyhow!("只有 Windows 支援 per-app 音訊裝置設定"))
}
