// aaa -- authentication, authorization and accounting on an IOS-like device (RFC 2865 / 2866
// behaviour towards the servers): the AAA configuration, the RADIUS servers and their groups,
// the method lists, and the running login sessions on the console and the vty lines.  The NAS
// side of RADIUS (sending Access-Requests, retransmitting, trying the next server) is the
// internal service "radius-nas" in radiusd.rs, which uses its own socket and timers; auth.rs
// holds the local secrets and users this builds on
//
//   Method                    group radius | group NAME | local | line | enable | none
//   MethodList { name, methods }   "aaa authentication login {default|LIST} METHOD..."
//   RadiusServer              a server by name: address, auth and acct ports, shared secret,
//                             timeout, retransmit (the older "radius-server host" makes one too)
//   ServerGroup { name, servers }   "aaa group server radius NAME" with "server name NAME"
//   Aaa                       the device's AAA state: new-model, the servers and groups, the
//                             login and enable method lists, exec authorization and accounting;
//                             list_for(line) / servers_of(method) resolve a line's methods and
//                             a method's servers
//   AuthResult                what a method decided: Pass(level) | Fail | NoServer | Continue
//
// Without "aaa new-model" a line falls back to the old rules (line password, "login local"):
// aaa.rs is consulted only when the model is on.  The next method of a list is tried only when
// a server does not answer (NoServer), never after a reject (Fail): that is the rule of
// authentication method lists.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use netpkt::Ipv4;

/// one authentication method
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Method {
    /// every configured RADIUS server (the default group)
    GroupRadius,
    /// a named server group
    Group(String),
    /// the device's local users (auth.rs)
    Local,
    /// the line's password
    Line,
    /// the enable secret / password
    Enable,
    /// let everyone in
    None,
}

impl Method {
    /// the word(s) of the running configuration
    pub fn words(&self) -> String {
        match self {
            Method::GroupRadius => String::from("group radius"),
            Method::Group(n) => format!("group {}", n),
            Method::Local => String::from("local"),
            Method::Line => String::from("line"),
            Method::Enable => String::from("enable"),
            Method::None => String::from("none"),
        }
    }
    pub fn parse(words: &[&str]) -> Option<(Method, usize)> {
        Some(match words {
            ["group", "radius", ..] => (Method::GroupRadius, 2),
            ["group", name, ..] => (Method::Group(String::from(*name)), 2),
            ["local", ..] => (Method::Local, 1),
            ["line", ..] => (Method::Line, 1),
            ["enable", ..] => (Method::Enable, 1),
            ["none", ..] => (Method::None, 1),
            _ => return None,
        })
    }
}

/// a named method list (its name is empty for "default")
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MethodList {
    pub name: String,
    pub methods: Vec<Method>,
}

impl MethodList {
    pub fn is_default(&self) -> bool {
        self.name.is_empty()
    }
    /// the name as the configuration writes it ("default" or the name)
    pub fn config_name(&self) -> &str {
        if self.name.is_empty() { "default" } else { &self.name }
    }
}

/// a RADIUS server the device may ask
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RadiusServer {
    /// the name of "radius server NAME" (empty for one made by "radius-server host")
    pub name: String,
    pub addr: Ipv4,
    pub auth_port: u16,
    pub acct_port: u16,
    pub key: String,
    /// seconds to wait for an answer
    pub timeout: u32,
    /// how many times to send the request again before giving up
    pub retransmit: u32,
}

impl RadiusServer {
    pub fn new(name: &str, addr: Ipv4) -> RadiusServer {
        RadiusServer { name: String::from(name), addr, auth_port: netpkt::radius::PORT_AUTH, acct_port: netpkt::radius::PORT_ACCT, key: String::new(), timeout: 5, retransmit: 3 }
    }
}

/// a named group of RADIUS servers
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServerGroup {
    pub name: String,
    /// the names of the servers in it, in order
    pub servers: Vec<String>,
}

/// exec accounting of "aaa accounting exec default ..."
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AcctExec {
    /// the list name (empty for default)
    pub name: String,
    /// "start-stop", "stop-only", "start-stop" is the usual one
    pub action: String,
    pub methods: Vec<Method>,
}

/// the AAA state of a device
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Aaa {
    pub new_model: bool,
    pub servers: Vec<RadiusServer>,
    pub groups: Vec<ServerGroup>,
    /// "aaa authentication login ..." lists
    pub login: Vec<MethodList>,
    /// "aaa authentication enable ..." lists (only "default" matters)
    pub enable: Vec<MethodList>,
    /// "aaa authorization exec default ..." methods (the privilege level from the server)
    pub author_exec: Option<MethodList>,
    /// "aaa accounting exec default ..."
    pub acct_exec: Option<AcctExec>,
}

impl Aaa {
    /// a login method list by name ("" is default)
    pub fn login_list(&self, name: &str) -> Option<&MethodList> {
        self.login.iter().find(|l| l.name == name)
    }
    /// the login methods a line with `list` uses: the named list, else the default list
    pub fn login_for<'a>(&'a self, list: &str) -> Option<&'a MethodList> {
        if !list.is_empty() {
            self.login_list(list).or_else(|| self.login_list(""))
        } else {
            self.login_list("")
        }
    }
    /// the enable "default" method list
    pub fn enable_list(&self) -> Option<&MethodList> {
        self.enable.iter().find(|l| l.name.is_empty())
    }
    pub fn server(&self, name: &str) -> Option<&RadiusServer> {
        self.servers.iter().find(|s| s.name == name)
    }
    pub fn server_mut(&mut self, name: &str) -> Option<&mut RadiusServer> {
        self.servers.iter_mut().find(|s| s.name == name)
    }
    pub fn group(&self, name: &str) -> Option<&ServerGroup> {
        self.groups.iter().find(|g| g.name == name)
    }
    /// the servers a method asks, in order (group radius: every server; a named group: its
    /// listed servers that exist)
    pub fn servers_of(&self, method: &Method) -> Vec<RadiusServer> {
        match method {
            Method::GroupRadius => self.servers.clone(),
            Method::Group(n) => self.group(n).map(|g| g.servers.iter().filter_map(|s| self.server(s).cloned()).collect()).unwrap_or_default(),
            _ => Vec::new(),
        }
    }
    /// set or replace a login list (empty methods removes it)
    pub fn set_login(&mut self, name: &str, methods: Vec<Method>) {
        self.login.retain(|l| l.name != name);
        if !methods.is_empty() {
            self.login.push(MethodList { name: String::from(name), methods });
        }
    }
    pub fn set_enable_list(&mut self, methods: Vec<Method>) {
        self.enable.retain(|l| !l.name.is_empty());
        if !methods.is_empty() {
            self.enable.push(MethodList { name: String::new(), methods });
        }
    }
    pub fn is_empty(&self) -> bool {
        !self.new_model && self.servers.is_empty() && self.groups.is_empty() && self.login.is_empty() && self.enable.is_empty() && self.author_exec.is_none() && self.acct_exec.is_none()
    }
    /// the bytes it holds (keyframe::node_bytes)
    pub fn footprint(&self) -> usize {
        self.servers.iter().map(|s| s.name.len() + s.key.len() + 32).sum::<usize>()
            + self.groups.iter().map(|g| g.name.len() + g.servers.iter().map(|s| s.len() + 8).sum::<usize>()).sum::<usize>()
            + self.login.iter().chain(&self.enable).map(|l| l.name.len() + l.methods.len() * 16 + 16).sum::<usize>()
            + 128
    }
}

/// who is logging in (which session the NAS answer resumes)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Requester {
    Console,
    /// a vty line (telnetd.rs)
    Vty(u16),
    /// "test aaa group radius": the outcome is printed, no session changes
    Test,
}

/// what a single authentication method decided
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthResult {
    /// authenticated, at this privilege level
    Pass(u8),
    /// the credentials were rejected (stop; do not try the next method)
    Fail,
    /// no server answered (try the next method)
    NoServer,
    /// this method cannot decide here; go on to the next
    Continue,
}
