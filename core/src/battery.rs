//! Battery samples from IOKit `AppleSmartBattery`. These are the measured anchors.

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub ts: i64,
    pub mah: i64,
    pub max_mah: i64,
    pub pct: i64,
    pub on_ac: bool,
}

/// Builds a sample from AppleSmartBattery properties; `get` returns a key's integer value
/// (booleans as 0/1). Apple silicon has AppleRaw* mAh keys and a percent CurrentCapacity.
pub fn from_props(ts: i64, get: impl Fn(&str) -> Option<i64>) -> Option<Sample> {
    let cur = get("CurrentCapacity")?;
    let on_ac = get("ExternalConnected").unwrap_or(0) != 0;
    if let (Some(mah), Some(max_mah)) = (get("AppleRawCurrentCapacity"), get("AppleRawMaxCapacity"))
    {
        return Some(Sample {
            ts,
            mah,
            max_mah,
            pct: cur,
            on_ac,
        });
    }
    // shortcut: Intel branch (no AppleRaw* keys; CurrentCapacity/MaxCapacity are mAh there).
    // Written to spec but untested on real Intel hardware.
    let max_mah = get("MaxCapacity").filter(|&m| m > 100)?;
    Some(Sample {
        ts,
        mah: cur,
        max_mah,
        pct: (cur * 100 + max_mah / 2) / max_mah,
        on_ac,
    })
}

/// Reads the battery now. None on Macs without a battery.
#[cfg(target_os = "macos")]
pub fn read(ts: i64) -> Option<Sample> {
    use core_foundation::base::{CFType, TCFType, kCFAllocatorDefault};
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::{CFDictionary, CFMutableDictionaryRef};
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use io_kit_sys::{
        IOObjectRelease, IORegistryEntryCreateCFProperties, IOServiceGetMatchingService,
        IOServiceMatching,
    };

    // SAFETY: plain IOKit calls; the props dictionary comes back +1 and is wrapped under the
    // create rule, the service object is released.
    let props: CFDictionary<CFString, CFType> = unsafe {
        let svc = IOServiceGetMatchingService(0, IOServiceMatching(c"AppleSmartBattery".as_ptr()));
        if svc == 0 {
            return None;
        }
        let mut raw: CFMutableDictionaryRef = std::ptr::null_mut();
        let kr = IORegistryEntryCreateCFProperties(svc, &mut raw, kCFAllocatorDefault, 0);
        IOObjectRelease(svc);
        if kr != 0 || raw.is_null() {
            return None;
        }
        CFDictionary::wrap_under_create_rule(raw as _)
    };
    let int = |v: &CFType| {
        v.downcast::<CFNumber>()
            .and_then(|n| n.to_i64())
            .or_else(|| v.downcast::<CFBoolean>().map(|b| bool::from(b) as i64))
    };
    // macOS 27 nests AppleRawCurrentCapacity/AppleRawMaxCapacity inside "BatteryData".
    let nested: Option<CFDictionary<CFString, CFType>> = props
        .find(CFString::from_static_string("BatteryData"))
        .and_then(|v| v.downcast::<CFDictionary>())
        // SAFETY: same object, typed view; get rule retains it.
        .map(|d| unsafe { CFDictionary::wrap_under_get_rule(d.as_concrete_TypeRef()) });
    let get = |key: &str| {
        let k = CFString::new(key);
        props
            .find(&k)
            .and_then(|v| int(&v))
            .or_else(|| nested.as_ref()?.find(&k).and_then(|v| int(&v)))
    };
    from_props(ts, get)
}

#[cfg(not(target_os = "macos"))]
pub fn read(_ts: i64) -> Option<Sample> {
    None
}
