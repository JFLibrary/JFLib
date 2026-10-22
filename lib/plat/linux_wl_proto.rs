// linux_wl_proto -- Wayland interface descriptors (event tables) and requests; opcodes and
// signatures follow the upstream protocol XML                     [src/plat/linux/wl_proto.s]
//
// An interface descriptor names the interface and lists its events in opcode order, each
// with its signature and handler (None = ignore).  Interfaces with several roles in the
// client (callbacks, surfaces, buffers) get one descriptor per role, so each role has its
// own handlers.  A handler is called as h(&mut Wl, object id, &args, udata) with every
// argument decoded (linux_wl_wire.rs Args).
//
// Requests: Req { op, "name:sig", destroy }; Wire::send(id, &REQ, &[A::...]) marshals the
// arguments in signature order (A::N = new_id of a typed interface, A::B = the untyped new_id
// of wl_registry.bind).  Signature characters: i u f o n s a h, `?` = nullable prefix, N =
// the bind new_id.  A destructor makes the object a zombie until wl_display.delete_id.

use super::clip::*;
use super::input::*;
use super::popup::*;
use super::present::*;
use super::tablet::*;
use super::window::*;
use super::wire::{Args, wlh_delete_id, wlh_display_error, wlh_sync_done};
use super::Wl;

pub(crate) type H = fn(&mut Wl, u32, &Args, u64);

pub(crate) struct Ev {
    pub sig: &'static str,
    pub h: Option<H>,
}

pub(crate) struct Iface {
    pub name: &'static str,
    pub evs: &'static [Ev],
}

pub(crate) struct Req {
    pub op: u16,
    pub sig: &'static str,
    pub destroy: bool,
}

const fn rq(op: u16, sig: &'static str) -> Req {
    Req { op, sig, destroy: false }
}

const fn rd(op: u16, sig: &'static str) -> Req {
    Req { op, sig, destroy: true }
}

const fn ev(sig: &'static str, h: H) -> Ev {
    Ev { sig, h: Some(h) }
}

const fn ig(sig: &'static str) -> Ev {
    Ev { sig, h: None }
}

// ------------------------------------------------------------------ requests
pub(crate) const DISPLAY_SYNC: Req = rq(0, "sync:n");
pub(crate) const DISPLAY_GET_REGISTRY: Req = rq(1, "get_registry:n");
pub(crate) const REGISTRY_BIND: Req = rq(0, "bind:uN");
pub(crate) const COMPOSITOR_CREATE_SURFACE: Req = rq(0, "create_surface:n");
pub(crate) const SHM_CREATE_POOL: Req = rq(0, "create_pool:nhi");
pub(crate) const SHM_POOL_CREATE_BUFFER: Req = rq(0, "create_buffer:niiiiu");
pub(crate) const SHM_POOL_DESTROY: Req = rd(1, "destroy:");
pub(crate) const BUFFER_DESTROY: Req = rd(0, "destroy:");
pub(crate) const SURFACE_DESTROY: Req = rd(0, "destroy:");
pub(crate) const SURFACE_ATTACH: Req = rq(1, "attach:?oii");
pub(crate) const SURFACE_FRAME: Req = rq(3, "frame:n");
pub(crate) const SURFACE_COMMIT: Req = rq(6, "commit:");
pub(crate) const SURFACE_SET_BUFFER_SCALE: Req = rq(8, "set_buffer_scale:i");
pub(crate) const SURFACE_DAMAGE_BUFFER: Req = rq(9, "damage_buffer:iiii");
pub(crate) const SEAT_GET_POINTER: Req = rq(0, "get_pointer:n");
pub(crate) const SEAT_GET_KEYBOARD: Req = rq(1, "get_keyboard:n");
pub(crate) const POINTER_SET_CURSOR: Req = rq(0, "set_cursor:u?oii");
pub(crate) const POINTER_RELEASE: Req = rd(1, "release:");
pub(crate) const KEYBOARD_RELEASE: Req = rd(0, "release:");
pub(crate) const OUTPUT_RELEASE: Req = rd(0, "release:");
pub(crate) const XOM_GET_XDG_OUTPUT: Req = rq(1, "get_xdg_output:no"); // zxdg_output_manager_v1
pub(crate) const XDG_OUTPUT_DESTROY: Req = rd(0, "destroy:");
pub(crate) const WM_BASE_GET_XDG_SURFACE: Req = rq(2, "get_xdg_surface:no");
pub(crate) const WM_BASE_PONG: Req = rq(3, "pong:u");
pub(crate) const XDG_SURFACE_DESTROY: Req = rd(0, "destroy:");
pub(crate) const XDG_SURFACE_GET_TOPLEVEL: Req = rq(1, "get_toplevel:n");
pub(crate) const XDG_SURFACE_SET_WINDOW_GEOMETRY: Req = rq(3, "set_window_geometry:iiii");
pub(crate) const XDG_SURFACE_ACK_CONFIGURE: Req = rq(4, "ack_configure:u");
pub(crate) const TOPLEVEL_DESTROY: Req = rd(0, "destroy:");
pub(crate) const TOPLEVEL_SET_PARENT: Req = rq(1, "set_parent:?o");
pub(crate) const TOPLEVEL_SET_TITLE: Req = rq(2, "set_title:s");
pub(crate) const TOPLEVEL_SET_APP_ID: Req = rq(3, "set_app_id:s");
pub(crate) const TOPLEVEL_MOVE: Req = rq(5, "move:ou");
pub(crate) const TOPLEVEL_RESIZE: Req = rq(6, "resize:ouu");
pub(crate) const TOPLEVEL_SET_MAX_SIZE: Req = rq(7, "set_max_size:ii");
pub(crate) const TOPLEVEL_SET_MIN_SIZE: Req = rq(8, "set_min_size:ii");
pub(crate) const TOPLEVEL_SET_MAXIMIZED: Req = rq(9, "set_maximized:");
pub(crate) const TOPLEVEL_UNSET_MAXIMIZED: Req = rq(10, "unset_maximized:");
pub(crate) const TOPLEVEL_UNSET_FULLSCREEN: Req = rq(12, "unset_fullscreen:");
pub(crate) const TOPLEVEL_SET_MINIMIZED: Req = rq(13, "set_minimized:");
pub(crate) const DDM_CREATE_DATA_SOURCE: Req = rq(0, "create_data_source:n");
pub(crate) const DDM_GET_DATA_DEVICE: Req = rq(1, "get_data_device:no");
pub(crate) const DATA_SOURCE_OFFER: Req = rq(0, "offer:s");
pub(crate) const DATA_SOURCE_DESTROY: Req = rd(1, "destroy:");
pub(crate) const DATA_DEVICE_SET_SELECTION: Req = rq(1, "set_selection:?ou");
pub(crate) const DATA_OFFER_RECEIVE: Req = rq(1, "receive:sh");
pub(crate) const DATA_OFFER_DESTROY: Req = rd(2, "destroy:");
pub(crate) const DATA_OFFER_ACCEPT: Req = rq(0, "accept:u?s"); // K1: drag and drop
pub(crate) const DATA_OFFER_FINISH: Req = rq(3, "finish:");
pub(crate) const DATA_OFFER_SET_ACTIONS: Req = rq(4, "set_actions:uu");
pub(crate) const DATA_SOURCE_SET_ACTIONS: Req = rq(2, "set_actions:u"); // the app's drag (dnd.rs)
pub(crate) const DATA_DEVICE_START_DRAG: Req = rq(0, "start_drag:?oo?ou");
pub(crate) const VIEWPORTER_GET_VIEWPORT: Req = rq(1, "get_viewport:no");
pub(crate) const VIEWPORT_DESTROY: Req = rd(0, "destroy:");
pub(crate) const VIEWPORT_SET_DESTINATION: Req = rq(2, "set_destination:ii");
pub(crate) const FSM_GET_FRACTIONAL_SCALE: Req = rq(1, "get_fractional_scale:no");
pub(crate) const FS_DESTROY: Req = rd(0, "destroy:");
pub(crate) const CSM_GET_POINTER: Req = rq(1, "get_pointer:no");
pub(crate) const CSD_SET_SHAPE: Req = rq(1, "set_shape:uu");
pub(crate) const DM_GET_TOPLEVEL_DECORATION: Req = rq(1, "get_toplevel_decoration:no");
pub(crate) const TD_DESTROY: Req = rd(0, "destroy:");
pub(crate) const TD_SET_MODE: Req = rq(1, "set_mode:u");
pub(crate) const WM_DIALOG_GET_XDG_DIALOG: Req = rq(1, "get_xdg_dialog:no");
pub(crate) const DIALOG_DESTROY: Req = rd(0, "destroy:");
pub(crate) const DIALOG_SET_MODAL: Req = rq(1, "set_modal:");
// (U3) popups: xdg_positioner, xdg_popup (linux_wl_popup.rs)
pub(crate) const WM_BASE_CREATE_POSITIONER: Req = rq(1, "create_positioner:n");
pub(crate) const POSITIONER_DESTROY: Req = rd(0, "destroy:");
pub(crate) const POSITIONER_SET_SIZE: Req = rq(1, "set_size:ii");
pub(crate) const POSITIONER_SET_ANCHOR_RECT: Req = rq(2, "set_anchor_rect:iiii");
pub(crate) const POSITIONER_SET_ANCHOR: Req = rq(3, "set_anchor:u");
pub(crate) const POSITIONER_SET_GRAVITY: Req = rq(4, "set_gravity:u");
pub(crate) const POSITIONER_SET_CONSTRAINT_ADJUSTMENT: Req = rq(5, "set_constraint_adjustment:u");
pub(crate) const POSITIONER_SET_OFFSET: Req = rq(6, "set_offset:ii");
pub(crate) const XDG_SURFACE_GET_POPUP: Req = rq(2, "get_popup:n?oo");
pub(crate) const POPUP_DESTROY: Req = rd(0, "destroy:");
// (PN1) tablet-unstable-v2 (linux_wl_tablet.rs), the tool's cursor shape
pub(crate) const TABLET_MANAGER_GET_TABLET_SEAT: Req = rq(0, "get_tablet_seat:no");
pub(crate) const TABLET_DESTROY: Req = rd(0, "destroy:");
pub(crate) const TABLET_TOOL_DESTROY: Req = rd(1, "destroy:");
pub(crate) const CSM_GET_TABLET_TOOL: Req = rq(2, "get_tablet_tool_v2:no");
pub(crate) const CSD_DESTROY: Req = rd(0, "destroy:");

// ------------------------------------------------------------------ interfaces + events
pub(crate) static IFACE_WL_DISPLAY: Iface =
    Iface { name: "wl_display", evs: &[ev("error:ous", wlh_display_error), ev("delete_id:u", wlh_delete_id)] };

pub(crate) static IFACE_WL_REGISTRY: Iface =
    Iface { name: "wl_registry", evs: &[ev("global:usu", wlh_global), ev("global_remove:u", wlh_global_remove)] };

pub(crate) static IFACE_CB_SYNC: Iface = Iface { name: "wl_callback", evs: &[ev("done:u", wlh_sync_done)] };

pub(crate) static IFACE_CB_FRAME: Iface = Iface { name: "wl_callback", evs: &[ev("done:u", wlh_frame_done)] };

pub(crate) static IFACE_WL_COMPOSITOR: Iface = Iface { name: "wl_compositor", evs: &[] };

pub(crate) static IFACE_WL_SHM: Iface = Iface { name: "wl_shm", evs: &[ig("format:u")] };

pub(crate) static IFACE_WL_SHM_POOL: Iface = Iface { name: "wl_shm_pool", evs: &[] };

pub(crate) static IFACE_WL_BUFFER: Iface = Iface { name: "wl_buffer", evs: &[ev("release:", wlh_buffer_release)] };

pub(crate) static IFACE_CURSOR_BUFFER: Iface = Iface { name: "wl_buffer", evs: &[ig("release:")] };

pub(crate) static IFACE_WL_SURFACE: Iface = Iface {
    name: "wl_surface",
    evs: &[ev("enter:o", wlh_surface_enter), ev("leave:o", wlh_surface_leave),
           ev("preferred_buffer_scale:i", wlh_surface_pbs), ig("preferred_buffer_transform:u")],
};

pub(crate) static IFACE_CURSOR_SURFACE: Iface = Iface {
    name: "wl_surface",
    evs: &[ig("enter:o"), ig("leave:o"), ig("preferred_buffer_scale:i"), ig("preferred_buffer_transform:u")],
};

pub(crate) static IFACE_WL_SEAT: Iface = Iface { name: "wl_seat", evs: &[ev("capabilities:u", wlh_seat_caps), ig("name:s")] };

pub(crate) static IFACE_WL_POINTER: Iface = Iface {
    name: "wl_pointer",
    evs: &[ev("enter:uoff", wlh_ptr_enter), ev("leave:uo", wlh_ptr_leave), ev("motion:uff", wlh_ptr_motion),
           ev("button:uuuu", wlh_ptr_button), ev("axis:uuf", wlh_ptr_axis), ev("frame:", wlh_ptr_frame),
           ig("axis_source:u"), ig("axis_stop:uu"), ev("axis_discrete:ui", wlh_ptr_discrete),
           ev("axis_value120:ui", wlh_ptr_value120), ig("axis_relative_direction:uu")],
};

pub(crate) static IFACE_WL_KEYBOARD: Iface = Iface {
    name: "wl_keyboard",
    evs: &[ev("keymap:uhu", wlh_kb_keymap), ev("enter:uoa", wlh_kb_enter), ev("leave:uo", wlh_kb_leave),
           ev("key:uuuu", wlh_kb_key), ev("modifiers:uuuuu", wlh_kb_modifiers), ev("repeat_info:ii", wlh_kb_repeat)],
};

pub(crate) static IFACE_WL_OUTPUT: Iface = Iface {
    name: "wl_output",
    evs: &[ev("geometry:iiiiissi", wlh_output_geometry), ev("mode:uiii", wlh_output_mode), ev("done:", wlh_output_done),
           ev("scale:i", wlh_output_scale), ev("name:s", wlh_output_name), ig("description:s")],
};

pub(crate) static IFACE_ZXDG_OUTPUT_MANAGER: Iface = Iface { name: "zxdg_output_manager_v1", evs: &[] };

/// an output's logical position and size in the compositor's space (screen.rs monitors)
pub(crate) static IFACE_ZXDG_OUTPUT: Iface = Iface {
    name: "zxdg_output_v1",
    evs: &[ev("logical_position:ii", wlh_xo_pos), ev("logical_size:ii", wlh_xo_size), ig("done:"),
           ev("name:s", wlh_xo_name), ig("description:s")],
};

pub(crate) static IFACE_XDG_WM_BASE: Iface = Iface { name: "xdg_wm_base", evs: &[ev("ping:u", wlh_ping)] };

pub(crate) static IFACE_XDG_SURFACE: Iface = Iface { name: "xdg_surface", evs: &[ev("configure:u", wlh_xdg_configure)] };

pub(crate) static IFACE_XDG_TOPLEVEL: Iface = Iface {
    name: "xdg_toplevel",
    evs: &[ev("configure:iia", wlh_top_configure), ev("close:", wlh_top_close), ig("configure_bounds:ii"),
           ig("wm_capabilities:a")],
};

pub(crate) static IFACE_WL_DDM: Iface = Iface { name: "wl_data_device_manager", evs: &[] };

pub(crate) static IFACE_WL_DATA_DEVICE: Iface = Iface {
    name: "wl_data_device",
    evs: &[ev("data_offer:n", wlh_dd_offer), ev("enter:uoff?o", wlh_dd_enter), ev("leave:", wlh_dd_leave),
           ev("motion:uff", wlh_dd_motion), ev("drop:", wlh_dd_drop), ev("selection:?o", wlh_dd_selection)],
};

pub(crate) static IFACE_WL_DATA_OFFER: Iface = Iface {
    name: "wl_data_offer",
    evs: &[ev("offer:s", wlh_offer_mime), ev("source_actions:u", wlh_offer_src_actions), ev("action:u", wlh_offer_action)],
};

pub(crate) static IFACE_WL_DATA_SOURCE: Iface = Iface {
    name: "wl_data_source",
    evs: &[ig("target:?s"), ev("send:sh", wlh_src_send), ev("cancelled:", wlh_src_cancelled),
           ev("dnd_drop_performed:", wlh_src_dropped), ev("dnd_finished:", wlh_src_dnd_finished), ev("action:u", wlh_src_action)],
};

pub(crate) static IFACE_WP_VIEWPORTER: Iface = Iface { name: "wp_viewporter", evs: &[] };

pub(crate) static IFACE_WP_VIEWPORT: Iface = Iface { name: "wp_viewport", evs: &[] };

pub(crate) static IFACE_WP_FSM: Iface = Iface { name: "wp_fractional_scale_manager_v1", evs: &[] };

pub(crate) static IFACE_WP_FS: Iface =
    Iface { name: "wp_fractional_scale_v1", evs: &[ev("preferred_scale:u", wlh_frac_scale)] };

pub(crate) static IFACE_WP_CSM: Iface = Iface { name: "wp_cursor_shape_manager_v1", evs: &[] };

pub(crate) static IFACE_WP_CSD: Iface = Iface { name: "wp_cursor_shape_device_v1", evs: &[] };

pub(crate) static IFACE_ZXDG_DM: Iface = Iface { name: "zxdg_decoration_manager_v1", evs: &[] };

pub(crate) static IFACE_ZXDG_TD: Iface =
    Iface { name: "zxdg_toplevel_decoration_v1", evs: &[ev("configure:u", wlh_deco_configure)] };

pub(crate) static IFACE_XDG_WM_DIALOG: Iface = Iface { name: "xdg_wm_dialog_v1", evs: &[] };

pub(crate) static IFACE_XDG_DIALOG: Iface = Iface { name: "xdg_dialog_v1", evs: &[] };

// (U3) popup windows (linux_wl_popup.rs): udata = popup slot (buffers: slot * 2 + i)
pub(crate) static IFACE_POP_SURFACE: Iface = Iface {
    name: "wl_surface",
    evs: &[ig("enter:o"), ig("leave:o"), ig("preferred_buffer_scale:i"), ig("preferred_buffer_transform:u")],
};

pub(crate) static IFACE_POP_XSURF: Iface = Iface { name: "xdg_surface", evs: &[ev("configure:u", wlh_pop_xdg_configure)] };

pub(crate) static IFACE_XDG_POSITIONER: Iface = Iface { name: "xdg_positioner", evs: &[] };

pub(crate) static IFACE_XDG_POPUP: Iface = Iface {
    name: "xdg_popup",
    evs: &[ev("configure:iiii", wlh_pop_configure), ev("popup_done:", wlh_pop_done), ig("repositioned:u")],
};

pub(crate) static IFACE_POP_BUFFER: Iface = Iface { name: "wl_buffer", evs: &[ev("release:", wlh_pop_release)] };

// (PN1) tablet-unstable-v2 (linux_wl_tablet.rs)
pub(crate) static IFACE_TABLET_MGR: Iface = Iface { name: "zwp_tablet_manager_v2", evs: &[] };

pub(crate) static IFACE_TABLET_SEAT: Iface = Iface {
    name: "zwp_tablet_seat_v2",
    evs: &[ev("tablet_added:n", wlh_tab_added), ev("tool_added:n", wlh_tool_added), ig("pad_added:n")],
};

pub(crate) static IFACE_TABLET: Iface = Iface {
    name: "zwp_tablet_v2",
    evs: &[ig("name:s"), ig("id:uu"), ig("path:s"), ig("done:"), ev("removed:", wlh_tab_removed)],
};

pub(crate) static IFACE_TABLET_TOOL: Iface = Iface {
    name: "zwp_tablet_tool_v2",
    evs: &[ev("type:u", wlh_tool_type), ig("hardware_serial:uu"), ig("hardware_id_wacom:uu"),
           ev("capability:u", wlh_tool_cap), ig("done:"), ev("removed:", wlh_tool_removed),
           ev("proximity_in:uoo", wlh_tool_in), ev("proximity_out:", wlh_tool_out), ev("down:u", wlh_tool_down),
           ev("up:", wlh_tool_up), ev("motion:ff", wlh_tool_motion), ev("pressure:u", wlh_tool_pressure),
           ig("distance:u"), ev("tilt:ff", wlh_tool_tilt), ig("rotation:f"), ig("slider:i"), ig("wheel:fi"),
           ev("button:uuu", wlh_tool_button), ev("frame:u", wlh_tool_frame)],
};

/// the interfaces linuxtest's synthetic events can address by name (first match wins)
pub(crate) static ALL_IFACES: [&Iface; 8] = [&IFACE_WL_POINTER, &IFACE_WL_KEYBOARD, &IFACE_WL_SEAT, &IFACE_WL_OUTPUT,
                                              &IFACE_XDG_TOPLEVEL, &IFACE_WL_DATA_DEVICE, &IFACE_TABLET_SEAT,
                                              &IFACE_TABLET_TOOL];
