// vk_inst -- Vulkan instance and adapters
//
//   Instance::new() / with(&InstanceDesc) -> Result<Instance>
//        jfgui_GPU=0 (or "off"): Err(Disabled); the loader from jfgui_GPU_DLL or vulkan-1.dll
//        (Err(NoLoader) without it); a loader below Vulkan 1.1: Err(Unsupported); no driver:
//        Err(NoAdapter).  Instance extensions: VK_KHR_surface + VK_KHR_win32_surface when
//        present, VK_EXT_debug_utils for jfgui_GPU_VALIDATE / jfgui_GPU_DEBUG.
//   instance.adapters() -> Result<Vec<Adapter>>   every physical device: discrete, integrated,
//        virtual, CPU, other (stable within a kind); AdapterInfo::missing says why one is
//        unusable (Vulkan 1.1, a graphics + compute queue, timeline semaphores, dynamic
//        rendering)
//   instance.adapter() -> Result<Adapter>     jfgui_GPU_ADAPTER: an index into adapters() or a
//        part of the name (case-insensitive); otherwise the first usable adapter
//   instance.adapters_low_power(display) -> Result<Vec<Adapter>>   the adapters for showing
//        windows (the UI's output): no CPU, virtual or unusable ones; the one that drives the
//        display (vendor and device id, plat::win_display_adapter) first, integrated before
//        discrete (low_power_rank); jfgui_GPU_ADAPTER as for adapter()
//   adapter.can_present(&surface)            its queue can present to the surface
//   adapter.info() / features() / limits() / level()   (level: the highest it supports)
//   validation_messages() -> (errors, warnings)  counted by the debug messenger of every
//        instance of the process (tests fail on errors)
//
// Validation (jfgui_GPU_VALIDATE=1 or InstanceDesc::validate): VK_LAYER_KHRONOS_validation when
// it is installed (the Vulkan SDK; never required), with synchronization validation
// (VK_EXT_layer_settings "validate_sync", else VK_EXT_validation_features); warnings and errors
// go to stderr as "gpu: validation error: ...".  Without the layer a note is printed once and
// everything runs as without the variable.  jfgui_GPU_DEBUG=1: the messenger alone (driver
// messages, info level and up).

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use core::ffi::c_void;
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use crate::vk_ffi as vk;
use crate::vk_load::Lib;
use crate::{AdapterInfo, AdapterKind, Features, GpuError, InstanceDesc, Level, Limits, Result};

/// a VkResult as an error
pub(crate) fn err(r: vk::VkResult) -> GpuError {
    match r {
        vk::ERROR_OUT_OF_HOST_MEMORY | vk::ERROR_OUT_OF_DEVICE_MEMORY | vk::ERROR_OUT_OF_POOL_MEMORY => {
            GpuError::OutOfMemory
        }
        vk::ERROR_DEVICE_LOST => GpuError::DeviceLost,
        vk::ERROR_SURFACE_LOST_KHR => GpuError::SurfaceLost,
        vk::TIMEOUT => GpuError::Timeout,
        vk::NOT_READY => GpuError::NotReady,
        vk::ERROR_FEATURE_NOT_PRESENT => GpuError::Unsupported("a device feature"),
        vk::ERROR_EXTENSION_NOT_PRESENT => GpuError::Unsupported("an extension"),
        vk::ERROR_FORMAT_NOT_SUPPORTED => GpuError::Unsupported("the format"),
        vk::ERROR_INCOMPATIBLE_DRIVER => GpuError::NoAdapter,
        _ => GpuError::Failed(r),
    }
}

pub(crate) fn env_on(name: &str) -> bool {
    plat::env(name).is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("on") || v.eq_ignore_ascii_case("true"))
}

static VAL_ERRORS: AtomicU32 = AtomicU32::new(0);
static VAL_WARNINGS: AtomicU32 = AtomicU32::new(0);
static NO_LAYER_NOTE: AtomicBool = AtomicBool::new(false);

/// validation errors and warnings reported by the debug messenger so far (every instance of
/// the process; 0, 0 without jfgui_GPU_VALIDATE / jfgui_GPU_DEBUG)
pub fn validation_messages() -> (u32, u32) {
    (VAL_ERRORS.load(Ordering::Relaxed), VAL_WARNINGS.load(Ordering::Relaxed))
}

unsafe extern "system" fn debug_message(sev: u32, types: u32, data: *const vk::DebugUtilsMessengerCallbackDataEXT,
                                        _user: *mut c_void) -> vk::Bool32 {
    let what = if sev & vk::DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT != 0 {
        if types & vk::DEBUG_UTILS_MESSAGE_TYPE_VALIDATION_BIT_EXT != 0 {
            VAL_ERRORS.fetch_add(1, Ordering::Relaxed);
        }
        "error"
    } else if sev & vk::DEBUG_UTILS_MESSAGE_SEVERITY_WARNING_BIT_EXT != 0 {
        VAL_WARNINGS.fetch_add(1, Ordering::Relaxed);
        "warning"
    } else {
        "info"
    };
    let kind = if types & vk::DEBUG_UTILS_MESSAGE_TYPE_VALIDATION_BIT_EXT != 0 {
        "validation"
    } else if types & vk::DEBUG_UTILS_MESSAGE_TYPE_PERFORMANCE_BIT_EXT != 0 {
        "performance"
    } else {
        "driver"
    };
    let mut s = String::with_capacity(256);
    s.push_str("gpu: ");
    s.push_str(kind);
    s.push(' ');
    s.push_str(what);
    s.push_str(": ");
    // SAFETY: the messenger passes valid callback data whose message is a C string.
    unsafe {
        if !data.is_null() && !(*data).p_message.is_null() {
            let p = (*data).p_message;
            let mut n = 0;
            while *p.add(n) != 0 {
                n += 1;
            }
            s.push_str(core::str::from_utf8(core::slice::from_raw_parts(p, n)).unwrap_or("?"));
        }
    }
    s.push('\n');
    plat::write_stderr(s.as_bytes());
    0
}

pub(crate) struct InstShared {
    pub inst: vk::Instance,
    pub f: vk::InstanceFns,
    /// VK_KHR_surface and the platform's surface extension are enabled
    pub surface: bool,
    pub validate: bool,
    messenger: vk::Handle,
    _lib: Lib, // last: unloaded after the instance is destroyed
}

impl Drop for InstShared {
    fn drop(&mut self) {
        // SAFETY: every child object holds an Arc of this instance, so none is left.
        unsafe {
            if self.messenger != 0 {
                if let Some(d) = self.f.destroy_debug_utils_messenger_ext {
                    d(self.inst, self.messenger, core::ptr::null());
                }
            }
            (self.f.destroy_instance)(self.inst, core::ptr::null());
        }
    }
}

/// the GPU API of the system (Vulkan); create adapters and surfaces from it
#[derive(Clone)]
pub struct Instance {
    pub(crate) sh: Arc<InstShared>,
}

/// the names of the extensions (of a layer: name) the loader offers
fn instance_extensions(lf: &vk::LoaderFns, layer: *const u8) -> Vec<vk::ExtensionProperties> {
    let mut n = 0u32;
    // SAFETY: the two-call enumeration with a correctly sized buffer.
    unsafe {
        if (lf.enumerate_instance_extension_properties)(layer, &mut n, core::ptr::null_mut()) != vk::SUCCESS {
            return Vec::new();
        }
        let mut v = vec![vk::Zero::zero(); n as usize];
        let r = (lf.enumerate_instance_extension_properties)(layer, &mut n, v.as_mut_ptr());
        if r != vk::SUCCESS && r != vk::INCOMPLETE {
            return Vec::new();
        }
        v.truncate(n as usize);
        v
    }
}

fn has_ext(list: &[vk::ExtensionProperties], name: &[u8]) -> bool {
    let name = &name[..name.len() - 1];
    list.iter().any(|e| vk::cstr(&e.extension_name).as_bytes() == name)
}

const LAYER_VALIDATION: &[u8] = b"VK_LAYER_KHRONOS_validation\0";

impl Instance {
    /// the system's GPU API with the default settings
    pub fn new() -> Result<Instance> {
        Instance::with(&InstanceDesc::DEFAULT)
    }

    pub fn with(desc: &InstanceDesc) -> Result<Instance> {
        if plat::env("jfgui_GPU").is_some_and(|v| v == "0" || v.eq_ignore_ascii_case("off")) {
            return Err(GpuError::Disabled);
        }
        let dll = plat::env("jfgui_GPU_DLL").filter(|s| !s.is_empty());
        let lib = Lib::open(dll.as_deref().unwrap_or("vulkan-1.dll"))?;
        let lf = vk::LoaderFns::load(&|n| lib.gipa(0, n)).map_err(|_| GpuError::NoLoader)?;
        let mut ver = vk::make_api_version(1, 0, 0);
        if let Some(e) = lf.enumerate_instance_version {
            // SAFETY: plain out-parameter.
            unsafe { e(&mut ver) };
        }
        if ver < vk::API_VERSION_1_1 {
            return Err(GpuError::Unsupported("a Vulkan 1.1 loader"));
        }
        let exts = instance_extensions(&lf, core::ptr::null());
        let mut want = desc.validate || env_on("jfgui_GPU_VALIDATE");
        let debug = env_on("jfgui_GPU_DEBUG");
        if want && !layer_present(&lf) {
            if !NO_LAYER_NOTE.swap(true, Ordering::Relaxed) {
                plat::write_stderr(b"gpu: jfgui_GPU_VALIDATE=1 but VK_LAYER_KHRONOS_validation is not installed: no validation\n");
            }
            want = false;
        }
        let layer_exts = if want { instance_extensions(&lf, LAYER_VALIDATION.as_ptr()) } else { Vec::new() };
        let mut names: Vec<*const u8> = Vec::new();
        let surface = cfg!(windows) && has_ext(&exts, b"VK_KHR_surface\0") && has_ext(&exts, b"VK_KHR_win32_surface\0");
        if surface {
            names.push(b"VK_KHR_surface\0".as_ptr());
            names.push(b"VK_KHR_win32_surface\0".as_ptr());
        }
        let du = (want || debug) && (has_ext(&exts, b"VK_EXT_debug_utils\0") || has_ext(&layer_exts, b"VK_EXT_debug_utils\0"));
        if du {
            names.push(b"VK_EXT_debug_utils\0".as_ptr());
        }
        let settings_ext = want && has_ext(&layer_exts, b"VK_EXT_layer_settings\0");
        let features_ext = want && !settings_ext && has_ext(&layer_exts, b"VK_EXT_validation_features\0");
        if settings_ext {
            names.push(b"VK_EXT_layer_settings\0".as_ptr());
        }
        if features_ext {
            names.push(b"VK_EXT_validation_features\0".as_ptr());
        }
        // synchronization validation: "validate_sync" of the layer (or the older feature list)
        let on: vk::Bool32 = 1;
        let setting = vk::LayerSettingEXT {
            p_layer_name: LAYER_VALIDATION.as_ptr(),
            p_setting_name: b"validate_sync\0".as_ptr(),
            type_: vk::LAYER_SETTING_TYPE_BOOL32_EXT,
            value_count: 1,
            p_values: &on as *const u32 as *const c_void,
        };
        let settings = vk::LayerSettingsCreateInfoEXT {
            s_type: vk::STRUCTURE_TYPE_LAYER_SETTINGS_CREATE_INFO_EXT,
            setting_count: 1,
            p_settings: &setting,
            ..vk::Zero::zero()
        };
        let sync = vk::VALIDATION_FEATURE_ENABLE_SYNCHRONIZATION_VALIDATION_EXT;
        let vfeatures = vk::ValidationFeaturesEXT {
            s_type: vk::STRUCTURE_TYPE_VALIDATION_FEATURES_EXT,
            enabled_validation_feature_count: 1,
            p_enabled_validation_features: &sync,
            ..vk::Zero::zero()
        };
        let mut app_name: Vec<u8> = desc.app_name.bytes().filter(|&b| b != 0).collect();
        app_name.push(0);
        let app = vk::ApplicationInfo {
            s_type: vk::STRUCTURE_TYPE_APPLICATION_INFO,
            p_application_name: app_name.as_ptr(),
            p_engine_name: b"JFLib gpu\0".as_ptr(),
            engine_version: vk::make_api_version(0, rt::VERSION_MAJOR as u32, rt::VERSION_MINOR as u32),
            api_version: vk::API_VERSION_1_3,
            ..vk::Zero::zero()
        };
        let layers = [LAYER_VALIDATION.as_ptr()];
        let info = vk::InstanceCreateInfo {
            s_type: vk::STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
            p_next: if settings_ext {
                &settings as *const _ as *const c_void
            } else if features_ext {
                &vfeatures as *const _ as *const c_void
            } else {
                core::ptr::null()
            },
            p_application_info: &app,
            enabled_layer_count: want as u32,
            pp_enabled_layer_names: layers.as_ptr(),
            enabled_extension_count: names.len() as u32,
            pp_enabled_extension_names: names.as_ptr(),
            ..vk::Zero::zero()
        };
        let mut inst: vk::Instance = 0;
        // SAFETY: every pointer of info lives until the call returns.
        let r = unsafe { (lf.create_instance)(&info, core::ptr::null(), &mut inst) };
        if r != vk::SUCCESS {
            return Err(err(r));
        }
        let f = match vk::InstanceFns::load(&|n| lib.gipa(inst, n)) {
            Ok(f) => f,
            Err(_) => {
                let d = lib.gipa(inst, b"vkDestroyInstance\0");
                if d != 0 {
                    // SAFETY: vkDestroyInstance of the instance just created.
                    unsafe {
                        core::mem::transmute::<usize, unsafe extern "system" fn(vk::Instance, *const c_void)>(d)(
                            inst,
                            core::ptr::null(),
                        )
                    };
                }
                return Err(GpuError::Unsupported("a Vulkan 1.1 instance"));
            }
        };
        let mut messenger = 0;
        if let (true, Some(create)) = (du, f.create_debug_utils_messenger_ext) {
            let mut sev = vk::DEBUG_UTILS_MESSAGE_SEVERITY_WARNING_BIT_EXT | vk::DEBUG_UTILS_MESSAGE_SEVERITY_ERROR_BIT_EXT;
            if debug {
                sev |= vk::DEBUG_UTILS_MESSAGE_SEVERITY_INFO_BIT_EXT;
            }
            let mi = vk::DebugUtilsMessengerCreateInfoEXT {
                s_type: vk::STRUCTURE_TYPE_DEBUG_UTILS_MESSENGER_CREATE_INFO_EXT,
                message_severity: sev,
                message_type: vk::DEBUG_UTILS_MESSAGE_TYPE_GENERAL_BIT_EXT
                    | vk::DEBUG_UTILS_MESSAGE_TYPE_VALIDATION_BIT_EXT
                    | vk::DEBUG_UTILS_MESSAGE_TYPE_PERFORMANCE_BIT_EXT,
                pfn_user_callback: Some(debug_message),
                ..vk::Zero::zero()
            };
            // SAFETY: a valid create info; the callback is a static function.
            if unsafe { create(inst, &mi, core::ptr::null(), &mut messenger) } != vk::SUCCESS {
                messenger = 0;
            }
        }
        Ok(Instance { sh: Arc::new(InstShared { inst, f, surface, validate: want, messenger, _lib: lib }) })
    }

    /// the validation layer is active on this instance
    pub fn validating(&self) -> bool {
        self.sh.validate
    }

    /// every GPU of the system, discrete ones first (AdapterInfo::missing: unusable ones)
    pub fn adapters(&self) -> Result<Vec<Adapter>> {
        let sh = &self.sh;
        let mut n = 0u32;
        // SAFETY: the two-call enumeration.
        let r = unsafe { (sh.f.enumerate_physical_devices)(sh.inst, &mut n, core::ptr::null_mut()) };
        if r != vk::SUCCESS {
            return Err(err(r));
        }
        let mut pds = vec![0usize; n as usize];
        // SAFETY: pds holds n handles.
        let r = unsafe { (sh.f.enumerate_physical_devices)(sh.inst, &mut n, pds.as_mut_ptr()) };
        if r != vk::SUCCESS && r != vk::INCOMPLETE {
            return Err(err(r));
        }
        pds.truncate(n as usize);
        let rank = |k: AdapterKind| match k {
            AdapterKind::Discrete => 0,
            AdapterKind::Integrated => 1,
            AdapterKind::Virtual => 2,
            AdapterKind::Cpu => 3,
            AdapterKind::Other => 4,
        };
        Ok(by_rank(pds.into_iter().map(|pd| probe(sh, pd)).map(|a| (rank(a.info.kind), a)).collect()))
    }

    /// the adapter to use: jfgui_GPU_ADAPTER (index or name part), else the first usable one
    pub fn adapter(&self) -> Result<Adapter> {
        let list = self.adapters()?;
        if let Some(sel) = plat::env("jfgui_GPU_ADAPTER").filter(|s| !s.is_empty()) {
            let pick = match sel.parse::<usize>() {
                Ok(i) => list.into_iter().nth(i),
                Err(_) => {
                    let want = sel.to_ascii_lowercase();
                    list.into_iter().find(|a| a.info.name.to_ascii_lowercase().contains(&want))
                }
            };
            let a = pick.ok_or(GpuError::NoAdapter)?;
            return match a.info.missing {
                Some(why) => Err(GpuError::Unsupported(why)),
                None => Ok(a),
            };
        }
        list.into_iter().find(|a| a.info.missing.is_none()).ok_or(GpuError::NoAdapter)
    }

    /// the adapters for showing windows without waking more hardware than needed, the most
    /// frugal first: only integrated and discrete GPUs (a CPU or virtual adapter is no gain over
    /// the CPU path), the one that drives the display (vendor id, device id) before the others,
    /// integrated before discrete.  jfgui_GPU_ADAPTER picks one as in adapter().  Programs that
    /// want a GPU for themselves keep adapter().
    pub fn adapters_low_power(&self, display: Option<(u32, u32)>) -> Result<Vec<Adapter>> {
        if plat::env("jfgui_GPU_ADAPTER").is_some_and(|s| !s.is_empty()) {
            return self.adapter().map(|a| alloc::vec![a]);
        }
        let list: Vec<(u32, Adapter)> = self
            .adapters()?
            .into_iter()
            .filter(|a| a.info.missing.is_none())
            .filter_map(|a| {
                let drives = display.is_some_and(|(v, d)| a.info.vendor_id == v && a.info.device_id == d);
                low_power_rank(a.info.kind, drives).map(|r| (r, a))
            })
            .collect();
        if list.is_empty() {
            return Err(GpuError::NoAdapter);
        }
        Ok(by_rank(list))
    }
}

/// the adapters by rank, stable within a rank (a handful: every program with a UI links this,
/// a general sort of these large records takes 14 KiB of code)
fn by_rank(mut list: Vec<(u32, Adapter)>) -> Vec<Adapter> {
    let mut out = Vec::with_capacity(list.len());
    while !list.is_empty() {
        let mut best = 0;
        for i in 1..list.len() {
            if list[i].0 < list[best].0 {
                best = i;
            }
        }
        out.push(list.remove(best).1);
    }
    out
}

/// the order of adapters for showing windows, lower first (None: not for this): the adapter that
/// drives the display needs no copy across GPUs and is awake anyway; an integrated one before a
/// discrete one that would have to wake up
pub fn low_power_rank(kind: AdapterKind, drives_display: bool) -> Option<u32> {
    let k = match kind {
        AdapterKind::Integrated => 0,
        AdapterKind::Discrete => 1,
        _ => return None,
    };
    Some(if drives_display { k } else { 2 + k })
}

fn layer_present(lf: &vk::LoaderFns) -> bool {
    let mut n = 0u32;
    // SAFETY: the two-call enumeration.
    unsafe {
        if (lf.enumerate_instance_layer_properties)(&mut n, core::ptr::null_mut()) != vk::SUCCESS {
            return false;
        }
        let mut v: Vec<vk::LayerProperties> = vec![vk::Zero::zero(); n as usize];
        let r = (lf.enumerate_instance_layer_properties)(&mut n, v.as_mut_ptr());
        if r != vk::SUCCESS && r != vk::INCOMPLETE {
            return false;
        }
        v.truncate(n as usize);
        let name = &LAYER_VALIDATION[..LAYER_VALIDATION.len() - 1];
        v.iter().any(|l| vk::cstr(&l.layer_name).as_bytes() == name)
    }
}

/// device extensions the backend uses
#[derive(Clone, Copy, Default)]
pub(crate) struct Exts {
    pub swapchain: bool,
    pub timeline_khr: bool,
    pub dynamic_rendering_khr: bool,
    pub depth_stencil_resolve_khr: bool,
    pub create_renderpass2_khr: bool,
    pub driver_properties_khr: bool,
}

/// a GPU of the system
#[derive(Clone)]
pub struct Adapter {
    pub(crate) inst: Arc<InstShared>,
    pub(crate) pd: vk::PhysicalDevice,
    pub(crate) info: AdapterInfo,
    pub(crate) features: Features,
    pub(crate) limits: Limits,
    pub(crate) voll: bool,
    pub(crate) api: u32,
    pub(crate) family: u32,
    pub(crate) ts_bits: u32,
    pub(crate) exts: Exts,
    pub(crate) f10: [u32; vk::F10_COUNT],
    pub(crate) f11: [u32; vk::F11_COUNT],
    pub(crate) f12: [u32; vk::F12_COUNT],
    pub(crate) f13: [u32; vk::F13_COUNT],
    pub(crate) mem: vk::PhysicalDeviceMemoryProperties,
    pub(crate) raw: vk::PhysicalDeviceLimits,
}

impl Adapter {
    pub fn info(&self) -> &AdapterInfo {
        &self.info
    }
    /// what a device of the highest level would have
    pub fn features(&self) -> &Features {
        &self.features
    }
    pub fn limits(&self) -> &Limits {
        &self.limits
    }
    /// the highest level a device of this adapter supports
    pub fn level(&self) -> Level {
        if self.voll { Level::Voll } else { Level::Basis }
    }
    /// a device of this adapter can show images on the surface
    pub fn can_present(&self, surface: &crate::Surface) -> bool {
        let Some(support) = self.inst.f.get_physical_device_surface_support_khr else { return false };
        if !self.exts.swapchain {
            return false;
        }
        let mut ok = 0;
        // SAFETY: plain query with a queue family of this adapter and a live surface.
        let r = unsafe { support(self.pd, self.family, surface.0.h, &mut ok) };
        r == vk::SUCCESS && ok != 0
    }
}

fn vendor_name(id: u32) -> &'static str {
    match id {
        0x10DE => "NVIDIA",
        0x1002 | 0x1022 => "AMD",
        0x8086 => "Intel",
        0x13B5 => "ARM",
        0x5143 => "Qualcomm",
        0x1010 => "Imagination",
        0x106B => "Apple",
        0x1414 => "Microsoft",
        0x10005 => "Mesa",
        _ => "unknown",
    }
}

fn driver_version(vendor: u32, v: u32) -> String {
    match vendor {
        0x10DE => alloc::format!("{}.{:02}", v >> 22, (v >> 14) & 0xFF),
        0x8086 if cfg!(windows) => alloc::format!("{}.{}", v >> 14, v & 0x3FFF),
        _ => alloc::format!("{}.{}.{}", vk::api_major(v), vk::api_minor(v), vk::api_patch(v)),
    }
}

fn device_extensions(sh: &InstShared, pd: vk::PhysicalDevice) -> Vec<vk::ExtensionProperties> {
    let mut n = 0u32;
    // SAFETY: the two-call enumeration.
    unsafe {
        if (sh.f.enumerate_device_extension_properties)(pd, core::ptr::null(), &mut n, core::ptr::null_mut()) != vk::SUCCESS {
            return Vec::new();
        }
        let mut v = vec![vk::Zero::zero(); n as usize];
        let r = (sh.f.enumerate_device_extension_properties)(pd, core::ptr::null(), &mut n, v.as_mut_ptr());
        if r != vk::SUCCESS && r != vk::INCOMPLETE {
            return Vec::new();
        }
        v.truncate(n as usize);
        v
    }
}

/// everything about one physical device
fn probe(sh: &Arc<InstShared>, pd: vk::PhysicalDevice) -> Adapter {
    let f = &sh.f;
    let dexts = device_extensions(sh, pd);
    let ext = |n: &[u8]| has_ext(&dexts, n);
    let mut exts = Exts {
        swapchain: sh.surface && ext(b"VK_KHR_swapchain\0"),
        timeline_khr: ext(b"VK_KHR_timeline_semaphore\0"),
        dynamic_rendering_khr: ext(b"VK_KHR_dynamic_rendering\0"),
        depth_stencil_resolve_khr: ext(b"VK_KHR_depth_stencil_resolve\0"),
        create_renderpass2_khr: ext(b"VK_KHR_create_renderpass2\0"),
        driver_properties_khr: ext(b"VK_KHR_driver_properties\0"),
    };
    // properties: base, driver, subgroups, descriptor indexing
    let mut dip: vk::PhysicalDeviceDescriptorIndexingProperties = vk::Zero::zero();
    dip.s_type = vk::STRUCTURE_TYPE_PHYSICAL_DEVICE_DESCRIPTOR_INDEXING_PROPERTIES;
    let mut sub: vk::PhysicalDeviceSubgroupProperties = vk::Zero::zero();
    sub.s_type = vk::STRUCTURE_TYPE_PHYSICAL_DEVICE_SUBGROUP_PROPERTIES;
    let mut size_ctl: vk::PhysicalDeviceSubgroupSizeControlProperties = vk::Zero::zero();
    size_ctl.s_type = vk::STRUCTURE_TYPE_PHYSICAL_DEVICE_SUBGROUP_SIZE_CONTROL_PROPERTIES;
    let mut drv: vk::PhysicalDeviceDriverProperties = vk::Zero::zero();
    drv.s_type = vk::STRUCTURE_TYPE_PHYSICAL_DEVICE_DRIVER_PROPERTIES;
    let mut p2: vk::PhysicalDeviceProperties2 = vk::Zero::zero();
    p2.s_type = vk::STRUCTURE_TYPE_PHYSICAL_DEVICE_PROPERTIES_2;
    // the API version decides the chain: read the base properties first
    // SAFETY: a valid structure without chain.
    unsafe { (f.get_physical_device_properties2)(pd, &mut p2) };
    let api = p2.properties.api_version;
    let v12 = api >= vk::API_VERSION_1_2;
    let has_drv = v12 || exts.driver_properties_khr;
    if api >= vk::API_VERSION_1_3 {
        dip.p_next = &mut size_ctl as *mut _ as *mut c_void;
    }
    sub.p_next = if v12 { &mut dip as *mut _ as *mut c_void } else { core::ptr::null_mut() };
    drv.p_next = &mut sub as *mut _ as *mut c_void;
    p2.p_next = if has_drv { &mut drv as *mut _ as *mut c_void } else { &mut sub as *mut _ as *mut c_void };
    // SAFETY: the chain's structures live on this stack frame.
    unsafe { (f.get_physical_device_properties2)(pd, &mut p2) };
    let props = p2.properties;
    let lim = props.limits;

    // features: Vulkan 1.0 + 1.1/1.2/1.3 structures, or the extension structures on 1.1
    let mut f13: vk::PhysicalDeviceVulkan13Features = vk::Zero::zero();
    f13.s_type = vk::STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_3_FEATURES;
    let mut f12: vk::PhysicalDeviceVulkan12Features = vk::Zero::zero();
    f12.s_type = vk::STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_2_FEATURES;
    let mut f11: vk::PhysicalDeviceVulkan11Features = vk::Zero::zero();
    f11.s_type = vk::STRUCTURE_TYPE_PHYSICAL_DEVICE_VULKAN_1_1_FEATURES;
    let mut dynr: vk::PhysicalDeviceOneFeature = vk::Zero::zero();
    dynr.s_type = vk::STRUCTURE_TYPE_PHYSICAL_DEVICE_DYNAMIC_RENDERING_FEATURES;
    let mut tl: vk::PhysicalDeviceOneFeature = vk::Zero::zero();
    tl.s_type = vk::STRUCTURE_TYPE_PHYSICAL_DEVICE_TIMELINE_SEMAPHORE_FEATURES;
    let mut f2: vk::PhysicalDeviceFeatures2 = vk::Zero::zero();
    f2.s_type = vk::STRUCTURE_TYPE_PHYSICAL_DEVICE_FEATURES_2;
    let v13 = api >= vk::API_VERSION_1_3;
    // linked from the tail: no structure is written after a pointer to it exists
    if v12 {
        if v13 {
            f12.p_next = &mut f13 as *mut _ as *mut c_void;
        } else if exts.dynamic_rendering_khr {
            f12.p_next = &mut dynr as *mut _ as *mut c_void;
        }
        f11.p_next = &mut f12 as *mut _ as *mut c_void;
        f2.p_next = &mut f11 as *mut _ as *mut c_void;
    } else {
        let mut head: *mut c_void = core::ptr::null_mut();
        if exts.dynamic_rendering_khr {
            head = &mut dynr as *mut _ as *mut c_void;
        }
        if exts.timeline_khr {
            tl.p_next = head;
            head = &mut tl as *mut _ as *mut c_void;
        }
        f2.p_next = head;
    }
    // SAFETY: the chain's structures live on this stack frame.
    unsafe { (f.get_physical_device_features2)(pd, &mut f2) };
    if !v12 {
        f12.f[vk::F12_TIMELINE_SEMAPHORE] = tl.f;
    }
    if !v13 {
        f13.f[vk::F13_DYNAMIC_RENDERING] = dynr.f;
    }
    if v13 {
        exts.dynamic_rendering_khr = false;
    }
    if v12 {
        exts.timeline_khr = false;
        exts.depth_stencil_resolve_khr = false;
        exts.create_renderpass2_khr = false;
    }

    // the queue: the first family with graphics and compute
    let mut nq = 0u32;
    // SAFETY: the two-call enumeration.
    unsafe { (f.get_physical_device_queue_family_properties)(pd, &mut nq, core::ptr::null_mut()) };
    let mut qf: Vec<vk::QueueFamilyProperties> = vec![vk::Zero::zero(); nq as usize];
    // SAFETY: qf holds nq entries.
    unsafe { (f.get_physical_device_queue_family_properties)(pd, &mut nq, qf.as_mut_ptr()) };
    qf.truncate(nq as usize);
    let gc = vk::QUEUE_GRAPHICS_BIT | vk::QUEUE_COMPUTE_BIT;
    let family = qf.iter().position(|q| q.queue_flags & gc == gc && q.queue_count > 0);
    let ts_bits = family.map_or(0, |i| qf[i].timestamp_valid_bits);

    let mut mem: vk::PhysicalDeviceMemoryProperties = vk::Zero::zero();
    // SAFETY: plain out-parameter.
    unsafe { (f.get_physical_device_memory_properties)(pd, &mut mem) };

    let b10 = |i: usize| f2.f[i] != 0;
    let b11 = |i: usize| v12 && f11.f[i] != 0;
    let b12 = |i: usize| f12.f[i] != 0;
    // on 1.1 the extension also needs VK_KHR_create_renderpass2 and VK_KHR_depth_stencil_resolve
    let dynamic_rendering = f13.f[vk::F13_DYNAMIC_RENDERING] != 0
        && (v12 || exts.create_renderpass2_khr && exts.depth_stencil_resolve_khr);
    let timeline = b12(vk::F12_TIMELINE_SEMAPHORE);
    let missing = if api < vk::API_VERSION_1_1 {
        Some("Vulkan 1.1")
    } else if family.is_none() {
        Some("a graphics + compute queue")
    } else if !timeline {
        Some("timeline semaphores")
    } else if !dynamic_rendering {
        Some("dynamic rendering")
    } else {
        None
    };
    let need_ops = vk::SUBGROUP_FEATURE_BASIC_BIT | vk::SUBGROUP_FEATURE_VOTE_BIT | vk::SUBGROUP_FEATURE_ARITHMETIC_BIT
        | vk::SUBGROUP_FEATURE_BALLOT_BIT | vk::SUBGROUP_FEATURE_SHUFFLE_BIT;
    let subgroups = sub.supported_stages & vk::SHADER_STAGE_COMPUTE_BIT != 0 && sub.supported_operations & need_ops == need_ops;
    let webgpu_ops = need_ops | vk::SUBGROUP_FEATURE_SHUFFLE_RELATIVE_BIT | vk::SUBGROUP_FEATURE_QUAD_BIT;
    let webgpu_stages = vk::SHADER_STAGE_COMPUTE_BIT | vk::SHADER_STAGE_FRAGMENT_BIT;
    let subgroups_webgpu = sub.supported_stages & webgpu_stages == webgpu_stages && sub.supported_operations & webgpu_ops == webgpu_ops;
    let mut subgroup_stages = 0;
    for (bit, stage) in [(vk::SHADER_STAGE_COMPUTE_BIT, crate::subgroup_stage::COMPUTE), (vk::SHADER_STAGE_FRAGMENT_BIT, crate::subgroup_stage::FRAGMENT), (vk::SHADER_STAGE_VERTEX_BIT, crate::subgroup_stage::VERTEX)] {
        if sub.supported_stages & bit != 0 {
            subgroup_stages |= stage;
        }
    }
    let subgroup_size_control = v13 && f13.f[vk::F13_SUBGROUP_SIZE_CONTROL] != 0 && size_ctl.required_subgroup_size_stages & vk::SHADER_STAGE_COMPUTE_BIT != 0;
    let bindless = v12
        && [
            vk::F12_DESCRIPTOR_INDEXING,
            vk::F12_RUNTIME_DESCRIPTOR_ARRAY,
            vk::F12_DESCRIPTOR_BINDING_PARTIALLY_BOUND,
            vk::F12_DESCRIPTOR_BINDING_UPDATE_UNUSED_WHILE_PENDING,
            vk::F12_DESCRIPTOR_BINDING_SAMPLED_IMAGE_UPDATE_AFTER_BIND,
            vk::F12_DESCRIPTOR_BINDING_STORAGE_IMAGE_UPDATE_AFTER_BIND,
            vk::F12_DESCRIPTOR_BINDING_STORAGE_BUFFER_UPDATE_AFTER_BIND,
            vk::F12_SHADER_SAMPLED_IMAGE_ARRAY_NON_UNIFORM_INDEXING,
            vk::F12_SHADER_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING,
        ]
        .iter()
        .all(|&i| b12(i))
        && b10(vk::F10_SHADER_SAMPLED_IMAGE_ARRAY_DYNAMIC_INDEXING)
        && b10(vk::F10_SHADER_STORAGE_BUFFER_ARRAY_DYNAMIC_INDEXING);
    let features = Features {
        timestamps: ts_bits > 0 && lim.timestamp_compute_and_graphics != 0,
        multi_draw_indirect: b10(vk::F10_MULTI_DRAW_INDIRECT),
        draw_indirect_first_instance: b10(vk::F10_DRAW_INDIRECT_FIRST_INSTANCE),
        draw_indirect_count: b12(vk::F12_DRAW_INDIRECT_COUNT) && v12,
        bindless,
        subgroups,
        subgroups_webgpu,
        subgroup_stages,
        subgroup_ops: sub.supported_operations,
        subgroup_size_control,
        subgroup_extended_types: v12 && b12(vk::F12_SHADER_SUBGROUP_EXTENDED_TYPES),
        draw_parameters: b11(vk::F11_SHADER_DRAW_PARAMETERS),
        shader_f16: v12 && b12(vk::F12_SHADER_FLOAT16),
        shader_f64: b10(vk::F10_SHADER_FLOAT64),
        shader_i16: b10(vk::F10_SHADER_INT16),
        shader_i64: b10(vk::F10_SHADER_INT64),
        shader_i8: v12 && b12(vk::F12_SHADER_INT8),
        storage_16bit: b11(vk::F11_STORAGE_BUFFER_16BIT_ACCESS),
        uniform_16bit: b11(vk::F11_UNIFORM_AND_STORAGE_BUFFER_16BIT_ACCESS),
        push_constant_16bit: b11(vk::F11_STORAGE_PUSH_CONSTANT16),
        uniform_standard_layout: v12 && b12(vk::F12_UNIFORM_BUFFER_STANDARD_LAYOUT),
        demote: v13 && f13.f[vk::F13_SHADER_DEMOTE_TO_HELPER_INVOCATION] != 0,
        clip_distances: b10(vk::F10_SHADER_CLIP_DISTANCE),
        primitive_index: b10(vk::F10_GEOMETRY_SHADER),
        sample_shading: b10(vk::F10_SAMPLE_RATE_SHADING),
        storage_8bit: v12 && b12(vk::F12_STORAGE_BUFFER_8BIT_ACCESS),
        texture_bc: b10(vk::F10_TEXTURE_COMPRESSION_BC),
        anisotropy: b10(vk::F10_SAMPLER_ANISOTROPY),
        depth_clamp: b10(vk::F10_DEPTH_CLAMP),
        vertex_stores: b10(vk::F10_VERTEX_PIPELINE_STORES_AND_ATOMICS),
        fragment_stores: b10(vk::F10_FRAGMENT_STORES_AND_ATOMICS),
        storage_write_without_format: b10(vk::F10_SHADER_STORAGE_IMAGE_WRITE_WITHOUT_FORMAT),
        buffer_address: v12 && b12(vk::F12_BUFFER_DEVICE_ADDRESS),
        swapchain: exts.swapchain,
        mesh_shader: ext(b"VK_EXT_mesh_shader\0"),
        ray_query: ext(b"VK_KHR_ray_query\0") && ext(b"VK_KHR_acceleration_structure\0"),
        ray_pipeline: ext(b"VK_KHR_ray_tracing_pipeline\0") && ext(b"VK_KHR_acceleration_structure\0"),
    };
    let voll = missing.is_none()
        && v13
        && bindless
        && features.draw_indirect_count
        && features.multi_draw_indirect
        && features.draw_indirect_first_instance
        && features.draw_parameters
        && subgroups;
    let uab = |set: usize, stage: usize| if bindless { dip.v[set].min(dip.v[stage]) } else { 0 };
    let device_memory = (0..mem.memory_heap_count.min(16) as usize)
        .filter(|&i| mem.memory_heaps[i].flags & 1 != 0)
        .map(|i| mem.memory_heaps[i].size)
        .sum();
    let limits = Limits {
        max_texture_2d: lim.max_image_dimension2_d,
        max_texture_3d: lim.max_image_dimension3_d,
        max_texture_layers: lim.max_image_array_layers,
        max_storage_binding: lim.max_storage_buffer_range,
        max_uniform_binding: lim.max_uniform_buffer_range,
        max_push_constants: lim.max_push_constants_size,
        max_bind_groups: lim.max_bound_descriptor_sets.min(crate::BIND_GROUPS_MAX),
        max_compute_workgroup_size: lim.max_compute_work_group_size,
        max_compute_invocations: lim.max_compute_work_group_invocations,
        max_compute_workgroups: lim.max_compute_work_group_count,
        max_compute_shared_memory: lim.max_compute_shared_memory_size,
        min_uniform_offset_alignment: lim.min_uniform_buffer_offset_alignment as u32,
        min_storage_offset_alignment: lim.min_storage_buffer_offset_alignment as u32,
        max_color_targets: lim.max_color_attachments.min(crate::COLOR_TARGETS_MAX as u32),
        max_vertex_buffers: lim.max_vertex_input_bindings,
        max_vertex_attributes: lim.max_vertex_input_attributes,
        max_draw_indirect_count: lim.max_draw_indirect_count,
        max_anisotropy: lim.max_sampler_anisotropy,
        timestamp_period_ns: lim.timestamp_period,
        subgroup_size: sub.subgroup_size,
        subgroup_min_size: if subgroup_size_control { size_ctl.min_subgroup_size } else { sub.subgroup_size },
        subgroup_max_size: if subgroup_size_control { size_ctl.max_subgroup_size } else { sub.subgroup_size },
        max_bindless_textures: uab(vk::DIP_MAX_SET_UAB_SAMPLED_IMAGES, vk::DIP_MAX_PER_STAGE_UAB_SAMPLED_IMAGES),
        max_bindless_samplers: uab(vk::DIP_MAX_SET_UAB_SAMPLERS, vk::DIP_MAX_PER_STAGE_UAB_SAMPLERS),
        max_bindless_buffers: uab(vk::DIP_MAX_SET_UAB_STORAGE_BUFFERS, vk::DIP_MAX_PER_STAGE_UAB_STORAGE_BUFFERS),
        max_bindless_storage_textures: uab(vk::DIP_MAX_SET_UAB_STORAGE_IMAGES, vk::DIP_MAX_PER_STAGE_UAB_STORAGE_IMAGES),
        device_memory,
    };
    let kind = match props.device_type {
        vk::PHYSICAL_DEVICE_TYPE_DISCRETE_GPU => AdapterKind::Discrete,
        vk::PHYSICAL_DEVICE_TYPE_INTEGRATED_GPU => AdapterKind::Integrated,
        vk::PHYSICAL_DEVICE_TYPE_VIRTUAL_GPU => AdapterKind::Virtual,
        vk::PHYSICAL_DEVICE_TYPE_CPU => AdapterKind::Cpu,
        _ => AdapterKind::Other,
    };
    let info = AdapterInfo {
        name: String::from(vk::cstr(&props.device_name)),
        vendor_id: props.vendor_id,
        device_id: props.device_id,
        vendor: vendor_name(props.vendor_id),
        kind,
        driver: if has_drv { String::from(vk::cstr(&drv.driver_name)) } else { String::new() },
        driver_info: if has_drv { String::from(vk::cstr(&drv.driver_info)) } else { String::new() },
        driver_version: driver_version(props.vendor_id, props.driver_version),
        api_version: (vk::api_major(api), vk::api_minor(api), vk::api_patch(api)),
        backend: "Vulkan",
        missing,
    };
    Adapter {
        inst: sh.clone(),
        pd,
        info,
        features,
        limits,
        voll,
        api,
        family: family.unwrap_or(0) as u32,
        ts_bits,
        exts,
        f10: f2.f,
        f11: f11.f,
        f12: f12.f,
        f13: f13.f,
        mem,
        raw: lim,
    }
}
