//! Windows Vulkan framebuffer presentation. CPU world/HUD rendering remains unchanged.
//! Transfer-only: no shader compiler/SDK, graphics framework, or emulation changes.
use ash::{vk, Entry};
use crate::presentation::{fit_rect, Timing};
use std::{ffi::{c_void, CStr}, time::Instant};
type Result<T> = std::result::Result<T,Box<dyn std::error::Error>>;
const TIMEOUT: u64 = 1_000_000_000;
const FORMAT: vk::Format = vk::Format::B8G8R8A8_UNORM;
static VALIDATION_ERRORS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
pub fn validation_errors() -> usize { VALIDATION_ERRORS.load(std::sync::atomic::Ordering::Relaxed) }

unsafe extern "system" fn debug_message(severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    _: vk::DebugUtilsMessageTypeFlagsEXT, data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>, _: *mut c_void) -> vk::Bool32 {
    if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
        VALIDATION_ERRORS.fetch_add(1,std::sync::atomic::Ordering::Relaxed);
    }
    if !data.is_null() && !(*data).p_message.is_null() {
        eprintln!("Vulkan validation: {}",CStr::from_ptr((*data).p_message).to_string_lossy());
    }
    vk::FALSE
}
#[derive(Default)]
struct Buffer { handle: vk::Buffer, memory: vk::DeviceMemory, mapped: *mut c_void, size: u64 }
#[derive(Default)]
struct Image { handle: vk::Image, memory: vk::DeviceMemory, size: (usize,usize) }
#[derive(Default)]
struct Chain {
    handle: vk::SwapchainKHR, images: Vec<vk::Image>, finished: Vec<vk::Semaphore>,
    fences: Vec<vk::Fence>, pending: Vec<bool>, extent: vk::Extent2D,
    fifo: bool, requested_vsync: bool, capture: bool, last_id: u64,
}

pub struct Renderer {
    _entry: Entry,
    instance: ash::Instance,
    debug: Option<ash::ext::debug_utils::Instance>, messenger: vk::DebugUtilsMessengerEXT,
    surface_api: ash::khr::surface::Instance, surface: vk::SurfaceKHR,
    physical: vk::PhysicalDevice, family: u32,
    device: Option<ash::Device>, queue: vk::Queue,
    swap_api: Option<ash::khr::swapchain::Device>, wait_api: Option<ash::khr::present_wait::Device>,
    chain: Chain, upload: Buffer, readback: Buffer, source: Image,
    pool: vk::CommandPool, cmd: vk::CommandBuffer, acquire: vk::Semaphore, fence: vk::Fence,
    queries: vk::QueryPool, timestamp_bits: u32, timestamp_period: f64, submitted: bool,
}
impl Renderer {
    pub fn new(hwnd: *mut c_void) -> Result<Self> { unsafe {
        #[link(name="kernel32")] extern "system" { fn GetModuleHandleW(name: *const u16) -> *mut c_void; }
        let hinstance = GetModuleHandleW(std::ptr::null());
        if hwnd.is_null() || hinstance.is_null() { return Err("invalid Vulkan window/module handle".into()); }
        let entry = Entry::load()?;
        let validation = std::env::var_os("CM_VK_VALIDATION").is_some();
        let layer = c"VK_LAYER_KHRONOS_validation";
        let mut extensions = vec![ash::khr::surface::NAME.as_ptr(),ash::khr::win32_surface::NAME.as_ptr(),
            ash::khr::get_surface_capabilities2::NAME.as_ptr(),ash::ext::surface_maintenance1::NAME.as_ptr()];
        let mut layers = Vec::new();
        if validation {
            if !entry.enumerate_instance_layer_properties()?.iter().any(|p| CStr::from_ptr(p.layer_name.as_ptr()) == layer) {
                return Err("CM_VK_VALIDATION requires the Khronos validation layer".into());
            }
            extensions.push(ash::ext::debug_utils::NAME.as_ptr());
            extensions.push(ash::ext::validation_features::NAME.as_ptr());
            layers.push(layer.as_ptr());
        }
        let app = vk::ApplicationInfo::default().application_name(c"CyberMage").api_version(vk::API_VERSION_1_1);
        let mut debug_info = vk::DebugUtilsMessengerCreateInfoEXT::default()
            .message_severity(vk::DebugUtilsMessageSeverityFlagsEXT::WARNING | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR)
            .message_type(vk::DebugUtilsMessageTypeFlagsEXT::GENERAL | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE)
            .pfn_user_callback(Some(debug_message));
        let features = [vk::ValidationFeatureEnableEXT::SYNCHRONIZATION_VALIDATION];
        let mut validation_info = vk::ValidationFeaturesEXT::default().enabled_validation_features(&features);
        let mut info = vk::InstanceCreateInfo::default().application_info(&app).enabled_extension_names(&extensions).enabled_layer_names(&layers);
        if validation { info = info.push_next(&mut debug_info).push_next(&mut validation_info); }
        let instance = entry.create_instance(&info,None)?;
        let surface_api = ash::khr::surface::Instance::new(&entry,&instance);
        // Construct the owner early: every following failed allocation is cleaned by Drop.
        let mut r = Self {
            _entry: entry, instance, debug: None, messenger: vk::DebugUtilsMessengerEXT::null(),
            surface_api, surface: vk::SurfaceKHR::null(), physical: vk::PhysicalDevice::null(), family: 0,
            device: None, queue: vk::Queue::null(), swap_api: None, wait_api: None,
            chain: Chain::default(), upload: Buffer::default(), readback: Buffer::default(), source: Image::default(),
            pool: vk::CommandPool::null(), cmd: vk::CommandBuffer::null(), acquire: vk::Semaphore::null(), fence: vk::Fence::null(),
            queries: vk::QueryPool::null(), timestamp_bits: 0, timestamp_period: 0.0, submitted: false,
        };
        if validation {
            r.debug = Some(ash::ext::debug_utils::Instance::new(&r._entry,&r.instance));
            r.messenger = r.debug.as_ref().unwrap().create_debug_utils_messenger(&debug_info,None)?;
        }
        r.surface = ash::khr::win32_surface::Instance::new(&r._entry,&r.instance).create_win32_surface(
            &vk::Win32SurfaceCreateInfoKHR::default().hinstance(hinstance as isize).hwnd(hwnd as isize),None)?;
        let required = [ash::khr::swapchain::NAME,ash::ext::swapchain_maintenance1::NAME,ash::khr::present_id::NAME,ash::khr::present_wait::NAME];
        let mut devices = r.instance.enumerate_physical_devices()?;
        devices.sort_by_key(|&p| if r.instance.get_physical_device_properties(p).device_type == vk::PhysicalDeviceType::DISCRETE_GPU {0} else {1});
        for physical in devices {
            let props = r.instance.get_physical_device_properties(physical);
            if props.api_version < vk::API_VERSION_1_1 { continue; }
            let available = r.instance.enumerate_device_extension_properties(physical)?;
            if !required.iter().all(|name| available.iter().any(|p| CStr::from_ptr(p.extension_name.as_ptr()) == *name)) { continue; }
            let mut maintenance = vk::PhysicalDeviceSwapchainMaintenance1FeaturesEXT::default();
            let mut id = vk::PhysicalDevicePresentIdFeaturesKHR::default();
            let mut wait = vk::PhysicalDevicePresentWaitFeaturesKHR::default();
            let mut features = vk::PhysicalDeviceFeatures2::default().push_next(&mut maintenance).push_next(&mut id).push_next(&mut wait);
            r.instance.get_physical_device_features2(physical,&mut features);
            if maintenance.swapchain_maintenance1 == 0 || id.present_id == 0 || wait.present_wait == 0 { continue; }
            for (family,q) in r.instance.get_physical_device_queue_family_properties(physical).iter().enumerate() {
                if q.queue_count == 0 || !q.queue_flags.contains(vk::QueueFlags::GRAPHICS) || !r.surface_api.get_physical_device_surface_support(physical,family as u32,r.surface)? { continue; }
                r.physical = physical; r.family = family as u32;
                r.timestamp_bits = q.timestamp_valid_bits; r.timestamp_period = props.limits.timestamp_period as f64;
                eprintln!("Vulkan device: {}",CStr::from_ptr(props.device_name.as_ptr()).to_string_lossy());
                break;
            }
            if r.physical != vk::PhysicalDevice::null() { break; }
        }
        if r.physical == vk::PhysicalDevice::null() { return Err("no graphics/present device supporting Vulkan 1.1, swapchain maintenance1 and present wait/id".into()); }
        let priorities = [1.0];
        let queues = [vk::DeviceQueueCreateInfo::default().queue_family_index(r.family).queue_priorities(&priorities)];
        let names: Vec<_> = required.iter().map(|s| s.as_ptr()).collect();
        let mut maintenance = vk::PhysicalDeviceSwapchainMaintenance1FeaturesEXT::default().swapchain_maintenance1(true);
        let mut id = vk::PhysicalDevicePresentIdFeaturesKHR::default().present_id(true);
        let mut wait = vk::PhysicalDevicePresentWaitFeaturesKHR::default().present_wait(true);
        let info = vk::DeviceCreateInfo::default().queue_create_infos(&queues).enabled_extension_names(&names)
            .push_next(&mut maintenance).push_next(&mut id).push_next(&mut wait);
        r.device = Some(r.instance.create_device(r.physical,&info,None)?);
        let d = r.device.as_ref().unwrap();
        r.queue = d.get_device_queue(r.family,0);
        r.swap_api = Some(ash::khr::swapchain::Device::new(&r.instance,d));
        r.wait_api = Some(ash::khr::present_wait::Device::new(&r.instance,d));
        r.pool = d.create_command_pool(&vk::CommandPoolCreateInfo::default().queue_family_index(r.family).flags(vk::CommandPoolCreateFlags::RESET_COMMAND_BUFFER),None)?;
        r.cmd = d.allocate_command_buffers(&vk::CommandBufferAllocateInfo::default().command_pool(r.pool).command_buffer_count(1))?[0];
        r.acquire = d.create_semaphore(&vk::SemaphoreCreateInfo::default(),None)?;
        r.fence = d.create_fence(&vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),None)?;
        if r.timestamp_bits > 0 { r.queries = d.create_query_pool(&vk::QueryPoolCreateInfo::default().query_type(vk::QueryType::TIMESTAMP).query_count(2),None)?; }
        Ok(r)
    } }
    pub fn paced(&self) -> bool { self.chain.handle != vk::SwapchainKHR::null() && self.chain.fifo }
    pub fn sync(&self) -> Result<()> { unsafe {
        if self.chain.last_id != 0 { self.wait_api.as_ref().unwrap().wait_for_present(self.chain.handle,self.chain.last_id,TIMEOUT)?; }
        Ok(())
    } }
    unsafe fn memory_type(&self, bits: u32, flags: vk::MemoryPropertyFlags) -> Result<u32> {
        let props = self.instance.get_physical_device_memory_properties(self.physical);
        (0..props.memory_type_count).find(|&i| bits & (1<<i) != 0 && props.memory_types[i as usize].property_flags.contains(flags))
            .ok_or_else(|| format!("Vulkan memory type unavailable: {flags:?}").into())
    }
    unsafe fn buffer(&self, size: u64, usage: vk::BufferUsageFlags) -> Result<Buffer> {
        let d = self.device.as_ref().unwrap();
        let mut b = Buffer::default();
        let result = (|| -> Result<()> {
            b.handle = d.create_buffer(&vk::BufferCreateInfo::default().size(size).usage(usage).sharing_mode(vk::SharingMode::EXCLUSIVE),None)?;
            let req = d.get_buffer_memory_requirements(b.handle);
            let ty = self.memory_type(req.memory_type_bits,vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT)?;
            b.memory = d.allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(ty),None)?;
            d.bind_buffer_memory(b.handle,b.memory,0)?;
            b.mapped = d.map_memory(b.memory,0,size,vk::MemoryMapFlags::empty())?; b.size = size;
            Ok(())
        })();
        if let Err(e) = result { self.destroy_buffer(&mut b); return Err(e); }
        Ok(b)
    }
    unsafe fn destroy_buffer(&self, b: &mut Buffer) {
        let d = self.device.as_ref().unwrap();
        if !b.mapped.is_null() { d.unmap_memory(b.memory); }
        d.destroy_buffer(b.handle,None); d.free_memory(b.memory,None); *b = Buffer::default();
    }
    unsafe fn source_image(&self, size: (usize,usize)) -> Result<Image> {
        let max = self.instance.get_physical_device_properties(self.physical).limits.max_image_dimension2_d as usize;
        if size.0 > max || size.1 > max { return Err("framebuffer exceeds Vulkan image limits".into()); }
        let d = self.device.as_ref().unwrap();
        let mut image = Image::default();
        let result = (|| -> Result<()> {
            let props = self.instance.get_physical_device_format_properties(self.physical,FORMAT);
            if !props.optimal_tiling_features.contains(vk::FormatFeatureFlags::BLIT_SRC) { return Err("BGRA8 Vulkan blit source unavailable".into()); }
            image.handle = d.create_image(&vk::ImageCreateInfo::default().image_type(vk::ImageType::TYPE_2D).format(FORMAT)
                .extent(vk::Extent3D {width:size.0 as u32,height:size.1 as u32,depth:1}).mip_levels(1).array_layers(1)
                .samples(vk::SampleCountFlags::TYPE_1).tiling(vk::ImageTiling::OPTIMAL)
                .usage(vk::ImageUsageFlags::TRANSFER_SRC | vk::ImageUsageFlags::TRANSFER_DST).sharing_mode(vk::SharingMode::EXCLUSIVE),None)?;
            let req = d.get_image_memory_requirements(image.handle);
            let ty = self.memory_type(req.memory_type_bits,vk::MemoryPropertyFlags::DEVICE_LOCAL)?;
            image.memory = d.allocate_memory(&vk::MemoryAllocateInfo::default().allocation_size(req.size).memory_type_index(ty),None)?;
            d.bind_image_memory(image.handle,image.memory,0)?; image.size = size; Ok(())
        })();
        if let Err(e) = result { d.destroy_image(image.handle,None); d.free_memory(image.memory,None); return Err(e); }
        Ok(image)
    }
    unsafe fn retire_chain(&mut self) -> Result<()> {
        let d = self.device.as_ref().unwrap();
        d.device_wait_idle()?;
        // Queue-submit completion alone does NOT prove the presentation engine is done
        // with its semaphores. maintenance1 fences explicitly retire those references.
        for (i,&pending) in self.chain.pending.iter().enumerate() {
            if pending { d.wait_for_fences(&[self.chain.fences[i]],true,u64::MAX)?; }
        }
        self.destroy_chain(); Ok(())
    }
    unsafe fn destroy_chain(&mut self) {
        let d = self.device.as_ref().unwrap();
        for s in self.chain.finished.drain(..) { d.destroy_semaphore(s,None); }
        for f in self.chain.fences.drain(..) { d.destroy_fence(f,None); }
        self.swap_api.as_ref().unwrap().destroy_swapchain(self.chain.handle,None);
        self.chain = Chain::default();
    }
    unsafe fn recreate(&mut self, window: (usize,usize), vsync: bool, capture: bool) -> Result<bool> {
        if self.chain.handle != vk::SwapchainKHR::null() && self.chain.extent.width as usize == window.0
            && self.chain.extent.height as usize == window.1 && self.chain.requested_vsync == vsync && self.chain.capture == capture { return Ok(true); }
        let caps = self.surface_api.get_physical_device_surface_capabilities(self.physical,self.surface)?;
        let extent = if caps.current_extent.width != u32::MAX { caps.current_extent } else { vk::Extent2D {
            width: (window.0 as u32).clamp(caps.min_image_extent.width,caps.max_image_extent.width),
            height: (window.1 as u32).clamp(caps.min_image_extent.height,caps.max_image_extent.height),
        } };
        if extent.width == 0 || extent.height == 0 { return Ok(false); }
        if self.chain.handle != vk::SwapchainKHR::null() && self.chain.extent == extent && self.chain.requested_vsync == vsync && self.chain.capture == capture { return Ok(true); }
        self.retire_chain()?;
        let formats = self.surface_api.get_physical_device_surface_formats(self.physical,self.surface)?;
        if !formats.iter().any(|f| (f.format == FORMAT || f.format == vk::Format::UNDEFINED) && f.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR) {
            return Err("surface lacks non-gamma-converting BGRA8 UNORM".into());
        }
        if !caps.supported_usage_flags.contains(vk::ImageUsageFlags::TRANSFER_DST) ||
            !caps.supported_composite_alpha.contains(vk::CompositeAlphaFlagsKHR::OPAQUE) ||
            !self.instance.get_physical_device_format_properties(self.physical,FORMAT).optimal_tiling_features.contains(vk::FormatFeatureFlags::BLIT_DST) {
            return Err("Vulkan surface lacks opaque transfer/blit presentation".into());
        }
        let modes = self.surface_api.get_physical_device_surface_present_modes(self.physical,self.surface)?;
        let mode = if vsync { vk::PresentModeKHR::FIFO } else if modes.contains(&vk::PresentModeKHR::IMMEDIATE) {
            vk::PresentModeKHR::IMMEDIATE
        } else if modes.contains(&vk::PresentModeKHR::MAILBOX) { vk::PresentModeKHR::MAILBOX } else { vk::PresentModeKHR::FIFO };
        let mut usage = vk::ImageUsageFlags::TRANSFER_DST;
        if capture && !caps.supported_usage_flags.contains(vk::ImageUsageFlags::TRANSFER_SRC) {
            return Err("surface does not support test readback".into());
        }
        self.chain.capture = capture;
        if capture { usage |= vk::ImageUsageFlags::TRANSFER_SRC; }
        let count = if caps.max_image_count == 0 { caps.min_image_count.max(2) } else { caps.min_image_count.max(2).min(caps.max_image_count) };
        let info = vk::SwapchainCreateInfoKHR::default().surface(self.surface).min_image_count(count)
            .image_format(FORMAT).image_color_space(vk::ColorSpaceKHR::SRGB_NONLINEAR).image_extent(extent).image_array_layers(1)
            .image_usage(usage).image_sharing_mode(vk::SharingMode::EXCLUSIVE).pre_transform(caps.current_transform)
            .composite_alpha(vk::CompositeAlphaFlagsKHR::OPAQUE).present_mode(mode).clipped(true);
        self.chain.handle = self.swap_api.as_ref().unwrap().create_swapchain(&info,None)?;
        self.chain.images = self.swap_api.as_ref().unwrap().get_swapchain_images(self.chain.handle)?;
        self.chain.extent = extent; self.chain.fifo = mode == vk::PresentModeKHR::FIFO; self.chain.requested_vsync = vsync;
        let d = self.device.as_ref().unwrap();
        for _ in &self.chain.images {
            self.chain.finished.push(d.create_semaphore(&vk::SemaphoreCreateInfo::default(),None)?);
            self.chain.fences.push(d.create_fence(&vk::FenceCreateInfo::default(),None)?);
        }
        self.chain.pending.resize(self.chain.images.len(),false);
        eprintln!("Vulkan swapchain: {}x{}, {mode:?}, {} images",extent.width,extent.height,self.chain.images.len());
        Ok(true)
    }
    unsafe fn barrier(&self, image: vk::Image, old: vk::ImageLayout, new: vk::ImageLayout,
        src: vk::AccessFlags, dst: vk::AccessFlags, from: vk::PipelineStageFlags, to: vk::PipelineStageFlags) {
        let barriers = [vk::ImageMemoryBarrier::default().image(image).old_layout(old).new_layout(new)
            .src_access_mask(src).dst_access_mask(dst).src_queue_family_index(vk::QUEUE_FAMILY_IGNORED).dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .subresource_range(vk::ImageSubresourceRange::default().aspect_mask(vk::ImageAspectFlags::COLOR).level_count(1).layer_count(1))];
        self.device.as_ref().unwrap().cmd_pipeline_barrier(self.cmd,from,to,vk::DependencyFlags::empty(),&[],&[],&barriers);
    }
    pub fn present(&mut self, pixels: &[u32], size: (usize,usize), window: (usize,usize), vsync: bool, capture: bool) -> Result<Timing> { unsafe {
        let bytes = size.0.checked_mul(size.1).and_then(|n| n.checked_mul(4)).ok_or("Vulkan framebuffer size overflow")?;
        if size.0 == 0 || size.1 == 0 || bytes/4 != pixels.len() || size.0 > i32::MAX as usize || size.1 > i32::MAX as usize {
            return Err("invalid Vulkan framebuffer dimensions/length".into());
        }
        if window.0 == 0 || window.1 == 0 { return Ok(Timing::default()); }
        let mut timing = Timing::default();
        let before = Instant::now();
        let d = self.device.as_ref().unwrap().clone();
        // ponytail: one GPU submission in flight; add a second only if measured throughput
        // warrants the extra buffering/latency. CPU simulation never moves onto this queue.
        d.wait_for_fences(&[self.fence],true,TIMEOUT)?;
        timing.wait += before.elapsed();
        if self.submitted && self.queries != vk::QueryPool::null() {
            let mut stamps = [0u64;2];
            d.get_query_pool_results(self.queries,0,&mut stamps,vk::QueryResultFlags::TYPE_64)?;
            let mask = u64::MAX >> (64-self.timestamp_bits);
            timing.gpu_ms = Some((stamps[1].wrapping_sub(stamps[0]) & mask) as f64*self.timestamp_period/1e6);
        }
        let before = Instant::now();
        if !self.recreate(window,vsync,capture)? { return Ok(timing); }
        timing.wait += before.elapsed();
        if self.source.size != size {
            d.destroy_image(self.source.handle,None); d.free_memory(self.source.memory,None); self.source = Image::default();
            self.source = self.source_image(size)?;
        }
        if self.upload.size < bytes as u64 {
            let mut b = std::mem::take(&mut self.upload); self.destroy_buffer(&mut b);
            self.upload = self.buffer(bytes as u64,vk::BufferUsageFlags::TRANSFER_SRC)?;
        }
        std::ptr::copy_nonoverlapping(pixels.as_ptr().cast::<u8>(),self.upload.mapped.cast::<u8>(),bytes);
        let before = Instant::now();
        let acquired = self.swap_api.as_ref().unwrap().acquire_next_image(self.chain.handle,TIMEOUT,self.acquire,vk::Fence::null());
        timing.wait += before.elapsed();
        let (index,suboptimal) = match acquired {
            Ok(v) => v,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => { self.retire_chain()?; return Ok(timing); }
            Err(e) => return Err(e.into()),
        };
        let i = index as usize;
        let before = Instant::now();
        if self.chain.pending[i] { d.wait_for_fences(&[self.chain.fences[i]],true,TIMEOUT)?; self.chain.pending[i] = false; }
        timing.wait += before.elapsed();
        let extent = self.chain.extent;
        if capture {
            if !self.chain.capture { return Err("surface does not support test readback".into()); }
            let n = extent.width as u64*extent.height as u64*4;
            if self.readback.size < n {
                let mut b = std::mem::take(&mut self.readback); self.destroy_buffer(&mut b);
                self.readback = self.buffer(n,vk::BufferUsageFlags::TRANSFER_DST)?;
            }
        }
        d.reset_command_buffer(self.cmd,vk::CommandBufferResetFlags::empty())?;
        d.begin_command_buffer(self.cmd,&vk::CommandBufferBeginInfo::default().flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT))?;
        if self.queries != vk::QueryPool::null() {
            d.cmd_reset_query_pool(self.cmd,self.queries,0,2);
            d.cmd_write_timestamp(self.cmd,vk::PipelineStageFlags::TOP_OF_PIPE,self.queries,0);
        }
        self.barrier(self.source.handle,vk::ImageLayout::UNDEFINED,vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::AccessFlags::empty(),vk::AccessFlags::TRANSFER_WRITE,vk::PipelineStageFlags::TOP_OF_PIPE,vk::PipelineStageFlags::TRANSFER);
        let layers = vk::ImageSubresourceLayers::default().aspect_mask(vk::ImageAspectFlags::COLOR).layer_count(1);
        let src_extent = vk::Extent3D { width:size.0 as u32,height:size.1 as u32,depth:1 };
        d.cmd_copy_buffer_to_image(self.cmd,self.upload.handle,self.source.handle,vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            &[vk::BufferImageCopy::default().image_subresource(layers).image_extent(src_extent)]);
        self.barrier(self.source.handle,vk::ImageLayout::TRANSFER_DST_OPTIMAL,vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            vk::AccessFlags::TRANSFER_WRITE,vk::AccessFlags::TRANSFER_READ,vk::PipelineStageFlags::TRANSFER,vk::PipelineStageFlags::TRANSFER);
        let target = self.chain.images[i];
        self.barrier(target,vk::ImageLayout::UNDEFINED,vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::AccessFlags::empty(),vk::AccessFlags::TRANSFER_WRITE,vk::PipelineStageFlags::TOP_OF_PIPE,vk::PipelineStageFlags::TRANSFER);
        let (x,y,w,h) = fit_rect(size,(extent.width as usize,extent.height as usize));
        if (x,y,w,h) != (0,0,extent.width as usize,extent.height as usize) {
            let range = vk::ImageSubresourceRange::default().aspect_mask(vk::ImageAspectFlags::COLOR).level_count(1).layer_count(1);
            d.cmd_clear_color_image(self.cmd,target,vk::ImageLayout::TRANSFER_DST_OPTIMAL,&vk::ClearColorValue {float32:[0.0,0.0,0.0,1.0]},&[range]);
            self.barrier(target,vk::ImageLayout::TRANSFER_DST_OPTIMAL,vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                vk::AccessFlags::TRANSFER_WRITE,vk::AccessFlags::TRANSFER_WRITE,vk::PipelineStageFlags::TRANSFER,vk::PipelineStageFlags::TRANSFER);
        }
        let blit = vk::ImageBlit::default().src_subresource(layers).dst_subresource(layers)
            .src_offsets([vk::Offset3D {x:0,y:0,z:0},vk::Offset3D {x:size.0 as i32,y:size.1 as i32,z:1}])
            .dst_offsets([vk::Offset3D {x:x as i32,y:y as i32,z:0},vk::Offset3D {x:(x+w) as i32,y:(y+h) as i32,z:1}]);
        if w != 0 && h != 0 {
            d.cmd_blit_image(self.cmd,self.source.handle,vk::ImageLayout::TRANSFER_SRC_OPTIMAL,target,vk::ImageLayout::TRANSFER_DST_OPTIMAL,&[blit],vk::Filter::NEAREST);
        }
        let mut layout = vk::ImageLayout::TRANSFER_DST_OPTIMAL;
        let mut access = vk::AccessFlags::TRANSFER_WRITE;
        if capture {
            self.barrier(target,layout,vk::ImageLayout::TRANSFER_SRC_OPTIMAL,access,vk::AccessFlags::TRANSFER_READ,vk::PipelineStageFlags::TRANSFER,vk::PipelineStageFlags::TRANSFER);
            layout = vk::ImageLayout::TRANSFER_SRC_OPTIMAL; access = vk::AccessFlags::TRANSFER_READ;
            d.cmd_copy_image_to_buffer(self.cmd,target,layout,self.readback.handle,&[vk::BufferImageCopy::default().image_subresource(layers)
                .image_extent(vk::Extent3D {width:extent.width,height:extent.height,depth:1})]);
            d.cmd_pipeline_barrier(self.cmd,vk::PipelineStageFlags::TRANSFER,vk::PipelineStageFlags::HOST,vk::DependencyFlags::empty(),
                &[vk::MemoryBarrier::default().src_access_mask(vk::AccessFlags::TRANSFER_WRITE).dst_access_mask(vk::AccessFlags::HOST_READ)],&[],&[]);
        }
        self.barrier(target,layout,vk::ImageLayout::PRESENT_SRC_KHR,access,vk::AccessFlags::empty(),vk::PipelineStageFlags::TRANSFER,vk::PipelineStageFlags::BOTTOM_OF_PIPE);
        if self.queries != vk::QueryPool::null() { d.cmd_write_timestamp(self.cmd,vk::PipelineStageFlags::BOTTOM_OF_PIPE,self.queries,1); }
        d.end_command_buffer(self.cmd)?;
        d.reset_fences(&[self.fence])?;
        let waits = [self.acquire]; let stages = [vk::PipelineStageFlags::TOP_OF_PIPE]; let cmds = [self.cmd]; let finished = [self.chain.finished[i]];
        let submit = vk::SubmitInfo::default().wait_semaphores(&waits).wait_dst_stage_mask(&stages).command_buffers(&cmds).signal_semaphores(&finished);
        d.queue_submit(self.queue,&[submit],self.fence)?; self.submitted = true;
        d.reset_fences(&[self.chain.fences[i]])?;
        let fences = [self.chain.fences[i]];
        let mut fence_info = vk::SwapchainPresentFenceInfoEXT::default().fences(&fences);
        let ids = [self.chain.last_id.checked_add(1).ok_or("Vulkan present ID overflow")?];
        let mut id_info = vk::PresentIdKHR::default().present_ids(&ids);
        let chains = [self.chain.handle]; let indices = [index];
        let present = vk::PresentInfoKHR::default().wait_semaphores(&finished).swapchains(&chains).image_indices(&indices)
            .push_next(&mut fence_info).push_next(&mut id_info);
        let before = Instant::now();
        let result = self.swap_api.as_ref().unwrap().queue_present(self.queue,&present);
        timing.wait += before.elapsed();
        match result {
            Ok(changed) => { self.chain.last_id = ids[0]; self.chain.pending[i] = true; if changed || suboptimal { self.retire_chain()?; } }
            // Out-of-date still enqueues the semaphore waits and their retirement fence.
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => { self.chain.pending[i] = true; self.retire_chain()?; }
            Err(vk::Result::ERROR_SURFACE_LOST_KHR) => {
                self.chain.pending[i] = true;
                return Err(vk::Result::ERROR_SURFACE_LOST_KHR.into());
            }
            Err(e) => return Err(e.into()),
        }
        Ok(timing)
    } }
    #[cfg(test)] fn captured(&self) -> Result<Vec<u32>> { unsafe {
        self.device.as_ref().unwrap().wait_for_fences(&[self.fence],true,TIMEOUT)?;
        let n = self.chain.extent.width as usize*self.chain.extent.height as usize;
        Ok(std::slice::from_raw_parts(self.readback.mapped.cast::<u32>(),n).to_vec())
    } }
}
impl Drop for Renderer {
    fn drop(&mut self) { unsafe {
        if self.device.is_some() {
            // Device-loss cannot be synchronized; otherwise retire presentation references.
            if let Err(e) = self.retire_chain() { eprintln!("Vulkan shutdown synchronization: {e}"); }
            // On device loss the device's children still require destruction.
            if self.chain.handle != vk::SwapchainKHR::null() { self.destroy_chain(); }
            let mut b = std::mem::take(&mut self.upload); self.destroy_buffer(&mut b);
            let mut b = std::mem::take(&mut self.readback); self.destroy_buffer(&mut b);
            let d = self.device.as_ref().unwrap();
            d.destroy_image(self.source.handle,None); d.free_memory(self.source.memory,None);
            d.destroy_query_pool(self.queries,None); d.destroy_fence(self.fence,None); d.destroy_semaphore(self.acquire,None);
            d.destroy_command_pool(self.pool,None); d.destroy_device(None);
        }
        self.surface_api.destroy_surface(self.surface,None);
        if let Some(debug) = &self.debug { debug.destroy_debug_utils_messenger(self.messenger,None); }
        self.instance.destroy_instance(None);
    } }
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] #[ignore = "requires a Windows Vulkan device and desktop; CM_VK_VALIDATION=1 enables synchronization validation"]
    fn vulkan_present_readback_resize_and_teardown() {
        let mut display = crate::display::Display::new(std::path::Path::new("__vulkan_test_no_settings__"));
        display.mode = 0; display.vulkan = true;
        let mut window = crate::frontend::create_window(&display,None).unwrap();
        assert!(window.is_vulkan(),"Vulkan test must not silently exercise software");
        let mut pixels: Vec<u32> = (0..640*480).map(|i| ((i%640) as u32 & 255)<<16 | ((i/640) as u32 & 255)<<8 | (i as u32 & 255)).collect();
        for on in [false,true,false] {
            let size = window.get_size();
            window.gpu.as_mut().unwrap().present(&pixels,(640,480),size,on,true).unwrap();
            let actual = window.gpu.as_ref().unwrap().captured().unwrap();
            assert_eq!(actual.iter().map(|p| p&0xffffff).collect::<Vec<_>>(),pixels);
            if on { window.gpu.as_ref().unwrap().sync().unwrap(); }
            pixels.reverse();
        }
        // Different source size exercises texture replacement and letterbox clearing.
        let size = window.get_size();
        window.gpu.as_mut().unwrap().present(&[0xff0000,0x00ff00,0x0000ff,0xffffff],(2,2),size,false,true).unwrap();
        let actual = window.gpu.as_ref().unwrap().captured().unwrap();
        let (x,y,w,h) = fit_rect((2,2),size);
        for row in 0..size.1 { for col in 0..size.0 {
            let expected = if col<x || col>=x+w || row<y || row>=y+h {0} else {
                [0xff0000,0x00ff00,0x0000ff,0xffffff][usize::from(row-y>=h/2)*2+usize::from(col-x>=w/2)]
            };
            assert_eq!(actual[row*size.0+col]&0xffffff,expected,"RGB/orientation/letterbox at {col},{row}");
        } }
        #[link(name="user32")] extern "system" { fn SetWindowPos(hwnd:*mut c_void,after:*mut c_void,x:i32,y:i32,w:i32,h:i32,flags:u32)->i32; }
        unsafe { assert_ne!(SetWindowPos(window.get_window_handle(),std::ptr::null_mut(),0,0,900,700,0x0002|0x0004),0); }
        window.update();
        let size = window.get_size();
        assert_ne!(size,(640,480));
        window.gpu.as_mut().unwrap().present(&pixels,(640,480),size,true,true).unwrap();
        window.gpu.as_ref().unwrap().sync().unwrap();
        assert_eq!(window.gpu.as_ref().unwrap().captured().unwrap().len(),size.0*size.1);
        drop(window);
        assert_eq!(validation_errors(),0);
    }
}
