use std::{fs, mem, path, ptr, sync, thread};
use std::collections::HashMap;
use std::time::Duration;
use errors::*;

use libc::*;

use core_foundation::array::CFArray;
use core_foundation::base::{CFType, CFTypeRef, TCFType};
use core_foundation::string::CFString;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::data::CFData;
use core_foundation::number::CFNumber;
use core_foundation::boolean::CFBoolean;
use core_foundation_sys::number::kCFBooleanTrue;

use mobiledevice_sys::*;
use ::{Device, PlatformManager};

#[derive(Clone,Debug)]
pub struct IosDevice {
    ptr: *const am_device,
    id: String,
    name: String,
    arch_cpu: &'static str,
}

unsafe impl Send for IosDevice {}

impl Device for IosDevice {
    fn name(&self) -> &str {
        &*self.name
    }
    fn id(&self) -> &str {
        &*self.id
    }
    fn target_arch(&self) -> &'static str {
        &*self.arch_cpu
    }
    fn target_vendor(&self) -> &'static str {
        "apple"
    }
    fn target_os(&self) -> &'static str {
        "ios"
    }
    fn start_remote_lldb(&self) -> Result<String> {
        let _ = ensure_session(self.ptr);
        let fd = start_remote_debug_server(self.ptr)?;
        debug!("start local lldb proxy");
        let proxy = start_lldb_proxy(fd)?;
        debug!("start lldb");
        Ok(format!("localhost:{}", proxy))
    }
    fn install_app(&self, app:&path::Path) -> Result<()> {
        install_app(self.ptr, app)
    }
    fn run_app(&self, app_path:&path::Path, app_id:&str, args:&str) -> Result<()> {
        let lldb_proxy = self.start_remote_lldb()?;
        run_remote(self.ptr, &lldb_proxy, app_path, app_id, args)
    }
}

impl IosDevice {
    fn from(ptr: *const am_device) -> Result<IosDevice> {
        let _session = ensure_session(ptr)?;
        let name = match device_read_value(ptr, "DeviceName")? {
            Some(Value::String(s)) => s,
            x => Err(format!("DeviceName should have been a string, was {:?}", x))?,
        };
        let cpu = match device_read_value(ptr, "CPUArchitecture")? {
            Some(Value::String(ref v)) if v == "arm64" => "aarch64",
            _ => "armv7"
        };
        let id =
            if let Value::String(id) = rustify(unsafe { AMDeviceCopyDeviceIdentifier(ptr) })? {
                id
            } else {
                Err("unexpected id format")?
            };
        Ok(IosDevice {
            ptr: ptr,
            name: name,
            id: id,
            arch_cpu: cpu.into()
        })
    }

}

pub struct IosManager {
    devices: sync::Arc<sync::Mutex<Vec<IosDevice>>>,
}

impl Default for IosManager {
    fn default() -> IosManager {
        let devices = sync::Arc::new(sync::Mutex::new(vec![]));

        let devices_to_take_away = Box::new(devices.clone());
        thread::spawn(move || {
            let notify: *const am_device_notification = ptr::null();
            unsafe {
                AMDeviceNotificationSubscribe(device_callback,
                                              0,
                                              0,
                                              Box::into_raw(devices_to_take_away) as *mut c_void,
                                              &mut notify.into());
            }
            ::core_foundation::runloop::CFRunLoop::run_current();
        });

        extern "C" fn device_callback(info: *mut am_device_notification_callback_info,
                                      devices: *mut c_void) {
            let device = unsafe { (*info).dev };
            let devices: &sync::Arc<sync::Mutex<Vec<IosDevice>>> =
                unsafe { mem::transmute(devices) };
            let _ = devices.lock()
                .map(|mut devices| devices.push(IosDevice::from(device).unwrap()));
        }

        IosManager {
            //       worker: thr,
            devices: devices,
        }
    }
}

impl PlatformManager for IosManager {
    fn devices(&self) -> Result<Vec<Box<Device>>> {
        let devices = self.devices.lock().map_err(|_| "poisoned lock")?;
        Ok(devices.iter().map(|d| Box::new(d.clone()) as Box<Device>).collect())
    }
}

#[derive(Clone,Debug)]
enum Value {
    String(String),
    Data(Vec<u8>),
    I64(i64),
    Boolean(bool),
}

fn mk_result(rv: i32) -> Result<()> {
    if rv != 0 {
        Err(format!("error: {:x}", rv))?
    } else {
        Ok(())
    }
}

fn rustify(raw: CFTypeRef) -> Result<Value> {

    unsafe {
        let cftype: CFType = TCFType::wrap_under_get_rule(mem::transmute(raw));
        if cftype.type_of() == CFString::type_id() {
            let value: CFString = TCFType::wrap_under_get_rule(mem::transmute(raw));
            return Ok(Value::String(value.to_string()));
        }

        if cftype.type_of() == CFData::type_id() {
            let value: CFData = TCFType::wrap_under_get_rule(mem::transmute(raw));
            return Ok(Value::Data(value.bytes().to_vec()));
        }
        if cftype.type_of() == CFNumber::type_id() {
            let value: CFNumber = TCFType::wrap_under_get_rule(mem::transmute(raw));
            if let Some(i) = value.to_i64() {
                return Ok(Value::I64(i));
            }
        }
        if cftype.type_of() == CFBoolean::type_id() {
            return Ok(Value::Boolean(raw == mem::transmute(kCFBooleanTrue)));
        }
        cftype.show();
        Err("unknown value")?
    }
}

fn device_read_value(dev: *const am_device, key: &str) -> Result<Option<Value>> {
    unsafe {
        let key = CFString::new(key);
        let raw = AMDeviceCopyValue(dev, ptr::null(), key.as_concrete_TypeRef());
        if raw.is_null() {
            return Ok(None);
        }
        Ok(Some(rustify(raw)?))
    }
}

fn xcode_dev_path() -> Result<path::PathBuf> {
    use std::process::Command;
    let command = Command::new("xcode-select").arg("-print-path").output()?;
    Ok(String::from_utf8(command.stdout)?.trim().into())
}

fn device_support_path(dev: *const am_device) -> Result<Option<path::PathBuf>> {
    let prefix = xcode_dev_path()?.join("Platforms/iPhoneOS.platform/DeviceSupport");
    let os_version = device_read_value(dev, "ProductVersion")?.ok_or("Could not get OS version")?;
    let two_token_version: String = if let Value::String(v) = os_version {
        v.split(".").take(2).collect::<Vec<_>>().join(".").into()
    } else {
        Err("ProductVersion should have be a string")?
    };
    for directory in fs::read_dir(&prefix)? {
        let directory = directory?;
        let last = directory.file_name()
            .into_string()
            .map_err(|d| format!("Could not parse {:?}", d))?;
        if last.starts_with(&two_token_version) {
            return Ok(Some(prefix.join(directory.path())));
        }
    }
    Ok(None)
}

fn mount_developper_image(dev: *const am_device) -> Result<()> {
    use std::io::Read;
    unsafe {
        let _session = ensure_session(dev);
        let ds_path = device_support_path(dev)?.ok_or("No device support found in xcode")?;
        let image_path = ds_path.join("DeveloperDiskImage.dmg");
        let sig_image_path = ds_path.join("DeveloperDiskImage.dmg.signature");
        let mut sig: Vec<u8> = vec![];
        fs::File::open(sig_image_path)?.read_to_end(&mut sig)?;
        let sig = CFData::from_buffer(&*sig);

        let options = [(CFString::from_static_string("ImageType"),
                        CFString::from_static_string("Developper").as_CFType()),
                       (CFString::from_static_string("ImageSignature"), sig.as_CFType())];
        let options = CFDictionary::from_CFType_pairs(&options);
        let r = AMDeviceMountImage(dev,
                                   CFString::new(image_path.to_str().unwrap())
                                       .as_concrete_TypeRef(),
                                   options.as_concrete_TypeRef(),
                                   ::std::mem::transmute(0usize),
                                   0);
        if r as u32 == 0xe8000076 {
            // already mounted, that's fine.
            return Ok(());
        }
        mk_result(r)?;
        Ok(())
    }
}

struct Session(*const am_device);

fn ensure_session(dev: *const am_device) -> Result<Session> {
    unsafe {
        debug!("ensure session 1");
        mk_result(AMDeviceConnect(dev))?;
        debug!("ensure session 1.4");
        if AMDeviceIsPaired(dev) == 0 {
            Err("lost pairing")?
        };
        debug!("ensure session 2");
        mk_result(AMDeviceValidatePairing(dev))?;
        debug!("ensure session 3");
        mk_result(AMDeviceStartSession(dev))?;
        Ok(Session(dev))
        /*
        debug!("ensure session 4 ({:x})", rv);
        if rv as u32 == 0xe800001d {
            Ok(Session(::std::ptr::null()))
        } else {
            mk_result(rv)?;
            Ok(Session(dev))
        }
        */
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            if !self.0.is_null() {
                if let Err(e) = mk_result(AMDeviceStopSession(self.0)) {
                    debug!("Error closing session {:?}", e);
                }
                if let Err(e) = mk_result(AMDeviceDisconnect(self.0)) {
                    error!("Error disconnecting {:?}", e);
                }
            }
        }
    }
}

pub fn install_app<P: AsRef<path::Path>>(dev: *const am_device, app: P) -> Result<()> {
    unsafe {
        let _session = ensure_session(dev)?;
        let path = app.as_ref().to_str().ok_or("failure to convert")?;
        let url =
            ::core_foundation::url::CFURL::from_file_system_path(CFString::new(path), 0, true);
        let options = [(CFString::from_static_string("PackageType"),
                        CFString::from_static_string("Developper").as_CFType())];
        let options = CFDictionary::from_CFType_pairs(&options);
        mk_result(AMDeviceSecureTransferPath(0,
                                             dev,
                                             url.as_concrete_TypeRef(),
                                             options.as_concrete_TypeRef(),
                                             ptr::null(),
                                             ptr::null()))?;
        mk_result(AMDeviceSecureInstallApplication(0,
                                                   dev,
                                                   url.as_concrete_TypeRef(),
                                                   options.as_concrete_TypeRef(),
                                                   ptr::null(),
                                                   ptr::null()))?;
    }
    Ok(())
}

fn start_remote_debug_server(dev: *const am_device) -> Result<c_int> {
    unsafe {
        debug!("mount developper image");
        mount_developper_image(dev)?;
        debug!("start debugserver on phone");
        let _session = ensure_session(dev)?;
        let mut fd: c_int = 0;
        mk_result(AMDeviceStartService(dev,
                                       CFString::from_static_string("com.apple.debugserver")
                                           .as_concrete_TypeRef(),
                                       &mut fd,
                                       ptr::null()))?;
        debug!("debug server running");
        Ok(fd)
    }
}

fn start_lldb_proxy(fd: c_int) -> Result<u16> {
    use std::net::{TcpStream, TcpListener};
    use std::os::unix::io::FromRawFd;
    use std::io::{Read, Write};
    let device = unsafe { TcpStream::from_raw_fd(fd) };
    let proxy = TcpListener::bind("127.0.0.1:0")?;
    let addr = proxy.local_addr()?;
    device.set_nonblocking(true)?;
    thread::spawn(move || {
        fn server(proxy: TcpListener, mut device: TcpStream) -> Result<()> {
            for stream in proxy.incoming() {
                let mut stream = stream.expect("Failure while accepting connection");
                stream.set_nonblocking(true)?;
                let mut buffer = [0; 1024];
                loop {
                    if let Ok(n) = stream.read(&mut buffer) {
                        if n == 0 {
                            break;
                        }
                        device.write_all(&buffer[0..n])?;
                    } else if let Ok(n) = device.read(&mut buffer) {
                        if n == 0 {
                            break;
                        }
                        stream.write_all(&buffer[0..n])?;
                    } else {
                        thread::sleep(Duration::from_millis(10));
                    }
                }
            }
            Ok(())
        }
        server(proxy, device).unwrap();
    });
    Ok(addr.port())
}

fn launch_lldb<P: AsRef<path::Path>, P2: AsRef<path::Path>>(dev: *const am_device,
                                                            proxy: &str,
                                                            local: P,
                                                            remote: P2,
                                                            args: &str)
                                                            -> Result<()> {
    use std::process::Command;
    use std::io::Write;
    let _session = ensure_session(dev);
    let dir = ::tempdir::TempDir::new("mobiledevice-rs-lldb")?;
    let tmppath = dir.path();
    let lldb_script_filename = tmppath.join("lldb-script");
    let sysroot = device_support_path(dev)
        ?
        .ok_or("no sysroot ?")?
        .to_str()
        .ok_or("could not read sysroot")?
        .to_owned();
    {
        let python_lldb_support = tmppath.join("helpers.py");
        fs::File::create(&python_lldb_support)?.write_all(include_bytes!("helpers.py"))?;
        let mut script = fs::File::create(&lldb_script_filename)?;
        writeln!(script, "platform select remote-ios --sysroot '{}'", sysroot)?;
        writeln!(script,
                 "target create {}",
                 local.as_ref().to_str().ok_or("untranslatable path")?)?;
        writeln!(script, "script pass")?;

        writeln!(script, "command script import {:?}", python_lldb_support)?;
        writeln!(script,
                 "command script add -f helpers.set_remote_path set_remote_path")?;
        writeln!(script,
                 "command script add -f helpers.connect_command connect")?;
        writeln!(script,
                 "command script add -s synchronous -f helpers.run_command run")?;

        writeln!(script, "connect connect://{}", proxy)?;
        writeln!(script,
                 "set_remote_path {}",
                 remote.as_ref().to_str().unwrap())?;
        writeln!(script, "run {}", args)?;
        writeln!(script, "quit")?;
    }

    Command::new("lldb").arg("-Q").arg("-s").arg(lldb_script_filename).status()?;
    Ok(())
}

pub fn run_remote<P: AsRef<path::Path>>(dev: *const am_device,
                                        lldb_proxy: &str,
                                        app_path: P,
                                        bundle_id: &str,
                                        args:&str)
                                        -> Result<()> {
    let _session = ensure_session(dev)?;

    let options = [(CFString::from_static_string("ReturnAttributes"),
                    CFArray::from_CFTypes(&[CFString::from_static_string("CFBundleIdentifier")
                                                .as_CFType(),
                                            CFString::from_static_string("Path").as_CFType()]))];
    let options = CFDictionary::from_CFType_pairs(&options);
    let apps: CFDictionaryRef = ptr::null();
    unsafe {
        mk_result(AMDeviceLookupApplications(dev,
                                             options.as_concrete_TypeRef(),
                                             ::std::mem::transmute(&apps)))?;
    }
    let apps: CFDictionary = unsafe { TCFType::wrap_under_get_rule(apps) };
    let app_info: CFDictionary =
        unsafe {
            TCFType::wrap_under_get_rule(::std::mem::transmute(
                apps.get(::std::mem::transmute(CFString::new(bundle_id).as_concrete_TypeRef()))
            ))
        };
    let remote: String = if let Ok(Value::String(remote)) = unsafe {
        rustify(app_info.get(::std::mem::transmute(CFString::from_static_string("Path")
            .as_concrete_TypeRef())))
    } {
        remote
    } else {
        Err("Invalid info")?
    };
    launch_lldb(dev, lldb_proxy, app_path, remote, args)?;
    Ok(())
}

#[allow(dead_code)]
fn properties(dev: *const am_device) -> Result<HashMap<&'static str, Value>> {
    let properties = ["ActivationPublicKey",
                      "ActivationState",
                      "ActivationStateAcknowledged",
                      "ActivityURL",
                      "BasebandBootloaderVersion",
                      "BasebandSerialNumber",
                      "BasebandStatus",
                      "BasebandVersion",
                      "BluetoothAddress",
                      "BuildVersion",
                      "CPUArchitecture",
                      "DeviceCertificate",
                      "DeviceClass",
                      "DeviceColor",
                      "DeviceName",
                      "DevicePublicKey",
                      "DieID",
                      "FirmwareVersion",
                      "HardwareModel",
                      "HardwarePlatform",
                      "HostAttached",
                      "IMLockdownEverRegisteredKey",
                      "IntegratedCircuitCardIdentity",
                      "InternationalMobileEquipmentIdentity",
                      "InternationalMobileSubscriberIdentity",
                      "iTunesHasConnected",
                      "MLBSerialNumber",
                      "MobileSubscriberCountryCode",
                      "MobileSubscriberNetworkCode",
                      "ModelNumber",
                      "PartitionType",
                      "PasswordProtected",
                      "PhoneNumber",
                      "ProductionSOC",
                      "ProductType",
                      "ProductVersion",
                      "ProtocolVersion",
                      "ProximitySensorCalibration",
                      "RegionInfo",
                      "SBLockdownEverRegisteredKey",
                      "SerialNumber",
                      "SIMStatus",
                      "SoftwareBehavior",
                      "SoftwareBundleVersion",
                      "SupportedDeviceFamilies",
                      "TelephonyCapability",
                      "TimeIntervalSince1970",
                      "TimeZone",
                      "TimeZoneOffsetFromUTC",
                      "TrustedHostAttached",
                      "UniqueChipID",
                      "UniqueDeviceID",
                      "UseActivityURL",
                      "UseRaptorCerts",
                      "Uses24HourClock",
                      "WeDelivered",
                      "WiFiAddress"];
    let mut props = HashMap::new();
    for p in properties.iter() {
        if let Some(v) = device_read_value(dev, p)? {
            props.insert(*p, v);
        }
    }
    Ok(props)
}
