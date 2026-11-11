//! Windows Core Audio：把指定錄音/播放裝置的音量拉到 100%。
//!
//! 為什麼需要：`Voicemeeter Out B1` 的 Windows 端音量預設只有 **24.3%**，
//! 訊號會被壓在 −21 dBFS，MyET 會判定「太小聲」而給低分。

#![cfg(windows)]

use anyhow::{anyhow, Result};
use windows::core::GUID;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{
    eCapture, eRender, IMMDevice, IMMDeviceCollection, IMMDeviceEnumerator, MMDeviceEnumerator,
    DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ,
};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, PROPERTYKEY};

/// PKEY_Device_FriendlyName = {a45c254e-df1c-4efd-8020-67d146a850e0}, 14
const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 14,
};

fn friendly_name(dev: &IMMDevice) -> Option<String> {
    unsafe {
        let store: IPropertyStore = dev.OpenPropertyStore(STGM_READ).ok()?;
        let pv = store.GetValue(&PKEY_DEVICE_FRIENDLY_NAME).ok()?;
        let pw = PropVariantToStringAlloc(&pv).ok()?;
        let s = pw.to_string().ok()?;
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }
}

fn enumerate(flow_capture: bool) -> Result<Vec<(IMMDevice, String)>> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let flow = if flow_capture { eCapture } else { eRender };
        let coll: IMMDeviceCollection = enumerator.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE)?;
        let n = coll.GetCount()?;
        let mut out = Vec::new();
        for i in 0..n {
            if let Ok(d) = coll.Item(i) {
                if let Some(name) = friendly_name(&d) {
                    out.push((d, name));
                }
            }
        }
        Ok(out)
    }
}

/// 列出所有裝置 (flow, 名稱, 音量 0~1)
pub fn list_endpoints() -> Result<Vec<(String, String, f32)>> {
    let mut out = Vec::new();
    for (capture, kind) in [(true, "capture"), (false, "render")] {
        for (dev, name) in enumerate(capture)? {
            let vol = unsafe {
                match dev.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None) {
                    Ok(iface) => iface.GetMasterVolumeLevelScalar().unwrap_or(-1.0),
                    Err(_) => -1.0,
                }
            };
            out.push((kind.to_string(), name, vol));
        }
    }
    Ok(out)
}

/// 把名稱含 `needle` 的裝置音量設為 `scalar`（1.0 = 100%）
pub fn set_volume(needle: &str, capture: bool, scalar: f32) -> Result<usize> {
    let needle_l = needle.to_lowercase();
    let mut hit = 0usize;
    for (dev, name) in enumerate(capture)? {
        if !name.to_lowercase().contains(&needle_l) {
            continue;
        }
        unsafe {
            let iface: IAudioEndpointVolume = dev.Activate(CLSCTX_ALL, None)?;
            iface.SetMute(false, std::ptr::null())?;
            iface.SetMasterVolumeLevelScalar(scalar, std::ptr::null())?;
            hit += 1;
            println!("  {name}: {:.1}%", iface.GetMasterVolumeLevelScalar()? * 100.0);
        }
    }
    if hit == 0 {
        return Err(anyhow!("找不到名稱含「{needle}」的裝置"));
    }
    Ok(hit)
}
