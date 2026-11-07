//! Bluetooth LE through WinRT: opening a lamp, finding its characteristics, notifications and
//! writes. Blocking calls (`join`), used from the engine thread (MTA).

use std::sync::mpsc::Sender;

use windows::{
    Devices::Bluetooth::{
        BluetoothCacheMode, BluetoothLEDevice,
        GenericAttributeProfile::{GattCharacteristic, GattCharacteristicProperties, GattClientCharacteristicConfigurationDescriptorValue, GattCommunicationStatus, GattDeviceService, GattValueChangedEventArgs, GattWriteOption},
    },
    Foundation::TypedEventHandler,
    Storage::Streams::{DataReader, DataWriter, IBuffer},
    core::{GUID, Ref},
};

pub fn es(e: windows::core::Error) -> String {
    format!("{} (0x{:08X})", e.message(), e.code().0)
}

pub fn buf_bytes(b: &IBuffer) -> windows::core::Result<Vec<u8>> {
    let r = DataReader::FromBuffer(b)?;
    let mut v = vec![0u8; b.Length()? as usize];
    r.ReadBytes(&mut v)?;
    Ok(v)
}

fn bytes_buf(v: &[u8]) -> windows::core::Result<IBuffer> {
    let w = DataWriter::new()?;
    w.WriteBytes(v)?;
    w.DetachBuffer()
}

/// An open device with all its characteristics.
pub struct Dev {
    pub dev: BluetoothLEDevice,
    pub name: String,
    services: Vec<GattDeviceService>,
    pub chars: Vec<(GUID, GattCharacteristic)>,
}

impl Drop for Dev {
    fn drop(&mut self) {
        for s in &self.services {
            let _ = s.Close();
        }
        let _ = self.dev.Close();
    }
}

impl Dev {
    pub fn find(&self, ids: &[GUID]) -> Option<GattCharacteristic> {
        ids.iter().find_map(|id| self.chars.iter().find(|c| c.0 == *id).map(|c| c.1.clone()))
    }
}

/// Opens the device at `addr` and reads its services (cache first, the device itself if the
/// cache is empty — the first connection after pairing in Windows).
pub fn open(addr: u64) -> Result<Dev, String> {
    let dev = BluetoothLEDevice::FromBluetoothAddressAsync(addr).and_then(|op| op.join()).map_err(es)?;
    let name = dev.Name().map(|n| n.to_string()).unwrap_or_default();
    let mut d = Dev { dev, name, services: Vec::new(), chars: Vec::new() };
    for mode in [BluetoothCacheMode::Cached, BluetoothCacheMode::Uncached] {
        let r = d.dev.GetGattServicesWithCacheModeAsync(mode).and_then(|op| op.join()).map_err(es)?;
        if r.Status().map_err(es)? != GattCommunicationStatus::Success {
            continue;
        }
        let Ok(list) = r.Services() else { continue };
        for s in list {
            if let Ok(cr) = s.GetCharacteristicsWithCacheModeAsync(mode).and_then(|op| op.join())
                && cr.Status().is_ok_and(|x| x == GattCommunicationStatus::Success)
                && let Ok(cs) = cr.Characteristics()
            {
                for c in cs {
                    if let Ok(u) = c.Uuid() {
                        d.chars.push((u, c));
                    }
                }
            }
            d.services.push(s);
        }
        if !d.chars.is_empty() {
            break;
        }
    }
    if d.chars.is_empty() {
        return Err("lamp does not answer".into());
    }
    Ok(d)
}

pub fn props(c: &GattCharacteristic) -> u32 {
    c.CharacteristicProperties().map(|p| p.0).unwrap_or(0)
}

/// Subscribes to notifications; every value goes to `tx` wrapped by `wrap`.
pub fn subscribe<M: Send + 'static>(c: &GattCharacteristic, tx: Sender<M>, wrap: fn(Vec<u8>, u64) -> M, generation: u64) -> Result<i64, String> {
    let token = c
        .ValueChanged(&TypedEventHandler::new(move |_, a: Ref<GattValueChangedEventArgs>| {
            let v = buf_bytes(&a.ok()?.CharacteristicValue()?)?;
            let _ = tx.send(wrap(v, generation));
            Ok(())
        }))
        .map_err(es)?;
    let p = props(c);
    let kind = if p & GattCharacteristicProperties::Notify.0 != 0 { GattClientCharacteristicConfigurationDescriptorValue::Notify } else { GattClientCharacteristicConfigurationDescriptorValue::Indicate };
    let st = c.WriteClientCharacteristicConfigurationDescriptorAsync(kind).and_then(|op| op.join()).map_err(es)?;
    if st != GattCommunicationStatus::Success {
        let _ = c.RemoveValueChanged(token);
        return Err(format!("notifications refused ({})", st.0));
    }
    Ok(token)
}

/// Writes one frame; without response only where the characteristic allows it.
pub fn write(c: &GattCharacteristic, data: &[u8], response: bool) -> Result<(), String> {
    let p = props(c);
    let nr = !response && p & GattCharacteristicProperties::WriteWithoutResponse.0 != 0 || p & GattCharacteristicProperties::Write.0 == 0;
    let opt = if nr { GattWriteOption::WriteWithoutResponse } else { GattWriteOption::WriteWithResponse };
    let st = c.WriteValueWithOptionAsync(&bytes_buf(data).map_err(es)?, opt).and_then(|op| op.join()).map_err(es)?;
    if st == GattCommunicationStatus::Success { Ok(()) } else { Err(format!("status {}", st.0)) }
}
