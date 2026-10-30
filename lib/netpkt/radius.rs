// radius -- RADIUS packets: authentication (RFC 2865), accounting (RFC 2866) and the
// Message-Authenticator (RFC 3579 3.2): codes, attributes by number and name, the hiding of
// User-Password (RFC 2865 5.2) both ways, the Request and Response Authenticators.  No digest
// is computed here (netpkt has no cryptography): the caller lends MD5 and HMAC-MD5, as
// crypto::md5_parts and crypto::hmac_md5 are
//
//   PORT_AUTH 1812, PORT_ACCT 1813, PORT_AUTH_OLD 1645, PORT_ACCT_OLD 1646 (old servers);
//   is_port(p)
//   ACCESS_REQUEST 1, ACCESS_ACCEPT 2, ACCESS_REJECT 3, ACCOUNTING_REQUEST 4,
//   ACCOUNTING_RESPONSE 5, ACCESS_CHALLENGE 11, STATUS_SERVER 12, STATUS_CLIENT 13; code_name
//   A_* the attributes of RFC 2865 (1 .. 39, 60 .. 63), RFC 2866 (40 .. 51) and RFC 3579 (79
//   EAP-Message, 80 Message-Authenticator); attr_name, attr_kind (Kind), value_name (the named
//   values of Service-Type, Framed-Protocol, Login-Service, NAS-Port-Type, Acct-Status-Type,
//   Acct-Authentic, Acct-Terminate-Cause, Termination-Action) and the values themselves
//   (SERVICE_*, PORT_TYPE_*, ACCT_*, TERM_*)
//   RadiusView::parse(b)            code, identifier, length, authenticator and the attributes
//                                   with their offsets (attrs()); Bad says why bytes are none
//   Packet { code, id, auth, attrs }    read(view) / parse(b) / to_bytes; attr / all / text /
//                                   int / addr of an attribute type
//   Attr { typ, value }             Attr::text / int / addr / bytes, Attr::vsa(vendor, data) and
//                                   vendor() (Vendor-Specific: the vendor's number and its data
//                                   kept as bytes)
//   hide_password(pw, secret, auth, md5) / reveal_password(hidden, secret, auth, md5)
//   response_auth(b, req_auth, secret, md5)   the Response Authenticator of a written answer;
//                                   set_response_auth / check_response_auth
//   accounting_auth(b, secret, md5) the Request Authenticator of an Accounting-Request
//                                   (RFC 2866 3); set_accounting_auth / check_accounting_auth
//   message_auth(b, auth, secret, hmac)   the Message-Authenticator over a packet (its field
//                                   counted as zero, `auth` in the authenticator field: the
//                                   packet's own for a request, the request's for an answer);
//                                   set_message_auth / check_message_auth (None: there is none)
//
// RFC 2865 3: a packet is 20 to 4096 bytes; octets beyond its Length field are padding, a
// packet shorter than its Length is discarded, and so is one whose attributes do not end
// exactly at Length.  Attribute values are at most 253 bytes.  A Message-Authenticator is
// added as 16 zero bytes (Attr::message_auth()) and filled in last but before the Response
// Authenticator, which covers it.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::addr::Ipv4;
use crate::{be16, be32, put16};

/// MD5 of the concatenation of the parts (what the caller lends: crypto::md5_parts)
pub type Md5Fn = fn(&[&[u8]]) -> [u8; 16];
/// HMAC-MD5 with a key over data (crypto::hmac_md5)
pub type HmacMd5Fn = fn(&[u8], &[u8]) -> [u8; 16];

pub const PORT_AUTH: u16 = 1812;
pub const PORT_ACCT: u16 = 1813;
/// the ports servers used before RFC 2865 (Cisco IOS still sends from them)
pub const PORT_AUTH_OLD: u16 = 1645;
pub const PORT_ACCT_OLD: u16 = 1646;

/// a port RADIUS uses
pub fn is_port(p: u16) -> bool {
    matches!(p, PORT_AUTH | PORT_ACCT | PORT_AUTH_OLD | PORT_ACCT_OLD)
}

pub const ACCESS_REQUEST: u8 = 1;
pub const ACCESS_ACCEPT: u8 = 2;
pub const ACCESS_REJECT: u8 = 3;
pub const ACCOUNTING_REQUEST: u8 = 4;
pub const ACCOUNTING_RESPONSE: u8 = 5;
pub const ACCESS_CHALLENGE: u8 = 11;
/// experimental (RFC 2865 3, RFC 5997)
pub const STATUS_SERVER: u8 = 12;
pub const STATUS_CLIENT: u8 = 13;

/// code, identifier, length, authenticator
pub const HDR: usize = 20;
/// the largest packet (RFC 2865 3)
pub const MAX: usize = 4096;
/// the longest attribute value
pub const VALUE_MAX: usize = 253;

/// the name of a code
pub fn code_name(c: u8) -> &'static str {
    match c {
        ACCESS_REQUEST => "Access-Request",
        ACCESS_ACCEPT => "Access-Accept",
        ACCESS_REJECT => "Access-Reject",
        ACCOUNTING_REQUEST => "Accounting-Request",
        ACCOUNTING_RESPONSE => "Accounting-Response",
        ACCESS_CHALLENGE => "Access-Challenge",
        STATUS_SERVER => "Status-Server",
        STATUS_CLIENT => "Status-Client",
        _ => "Unknown",
    }
}

// the attributes of RFC 2865 5
pub const A_USER_NAME: u8 = 1;
pub const A_USER_PASSWORD: u8 = 2;
pub const A_CHAP_PASSWORD: u8 = 3;
pub const A_NAS_IP_ADDRESS: u8 = 4;
pub const A_NAS_PORT: u8 = 5;
pub const A_SERVICE_TYPE: u8 = 6;
pub const A_FRAMED_PROTOCOL: u8 = 7;
pub const A_FRAMED_IP_ADDRESS: u8 = 8;
pub const A_FRAMED_IP_NETMASK: u8 = 9;
pub const A_FRAMED_ROUTING: u8 = 10;
pub const A_FILTER_ID: u8 = 11;
pub const A_FRAMED_MTU: u8 = 12;
pub const A_FRAMED_COMPRESSION: u8 = 13;
pub const A_LOGIN_IP_HOST: u8 = 14;
pub const A_LOGIN_SERVICE: u8 = 15;
pub const A_LOGIN_TCP_PORT: u8 = 16;
pub const A_REPLY_MESSAGE: u8 = 18;
pub const A_CALLBACK_NUMBER: u8 = 19;
pub const A_CALLBACK_ID: u8 = 20;
pub const A_FRAMED_ROUTE: u8 = 22;
pub const A_FRAMED_IPX_NETWORK: u8 = 23;
pub const A_STATE: u8 = 24;
pub const A_CLASS: u8 = 25;
pub const A_VENDOR_SPECIFIC: u8 = 26;
pub const A_SESSION_TIMEOUT: u8 = 27;
pub const A_IDLE_TIMEOUT: u8 = 28;
pub const A_TERMINATION_ACTION: u8 = 29;
pub const A_CALLED_STATION_ID: u8 = 30;
pub const A_CALLING_STATION_ID: u8 = 31;
pub const A_NAS_IDENTIFIER: u8 = 32;
pub const A_PROXY_STATE: u8 = 33;
pub const A_LOGIN_LAT_SERVICE: u8 = 34;
pub const A_LOGIN_LAT_NODE: u8 = 35;
pub const A_LOGIN_LAT_GROUP: u8 = 36;
pub const A_FRAMED_APPLETALK_LINK: u8 = 37;
pub const A_FRAMED_APPLETALK_NETWORK: u8 = 38;
pub const A_FRAMED_APPLETALK_ZONE: u8 = 39;
// RFC 2866 5
pub const A_ACCT_STATUS_TYPE: u8 = 40;
pub const A_ACCT_DELAY_TIME: u8 = 41;
pub const A_ACCT_INPUT_OCTETS: u8 = 42;
pub const A_ACCT_OUTPUT_OCTETS: u8 = 43;
pub const A_ACCT_SESSION_ID: u8 = 44;
pub const A_ACCT_AUTHENTIC: u8 = 45;
pub const A_ACCT_SESSION_TIME: u8 = 46;
pub const A_ACCT_INPUT_PACKETS: u8 = 47;
pub const A_ACCT_OUTPUT_PACKETS: u8 = 48;
pub const A_ACCT_TERMINATE_CAUSE: u8 = 49;
pub const A_ACCT_MULTI_SESSION_ID: u8 = 50;
pub const A_ACCT_LINK_COUNT: u8 = 51;
// RFC 2865 5 again
pub const A_CHAP_CHALLENGE: u8 = 60;
pub const A_NAS_PORT_TYPE: u8 = 61;
pub const A_PORT_LIMIT: u8 = 62;
pub const A_LOGIN_LAT_PORT: u8 = 63;
// RFC 3579
pub const A_EAP_MESSAGE: u8 = 79;
pub const A_MESSAGE_AUTHENTICATOR: u8 = 80;

/// Service-Type (RFC 2865 5.6)
pub const SERVICE_LOGIN: u32 = 1;
pub const SERVICE_FRAMED: u32 = 2;
pub const SERVICE_CALLBACK_LOGIN: u32 = 3;
pub const SERVICE_CALLBACK_FRAMED: u32 = 4;
pub const SERVICE_OUTBOUND: u32 = 5;
pub const SERVICE_ADMINISTRATIVE: u32 = 6;
pub const SERVICE_NAS_PROMPT: u32 = 7;
pub const SERVICE_AUTHENTICATE_ONLY: u32 = 8;
/// NAS-Port-Type (RFC 2865 5.41)
pub const PORT_TYPE_ASYNC: u32 = 0;
pub const PORT_TYPE_VIRTUAL: u32 = 5;
pub const PORT_TYPE_ETHERNET: u32 = 15;
/// Acct-Status-Type (RFC 2866 5.1)
pub const ACCT_START: u32 = 1;
pub const ACCT_STOP: u32 = 2;
pub const ACCT_INTERIM: u32 = 3;
pub const ACCT_ON: u32 = 7;
pub const ACCT_OFF: u32 = 8;
/// Acct-Authentic (RFC 2866 5.6)
pub const AUTHENTIC_RADIUS: u32 = 1;
pub const AUTHENTIC_LOCAL: u32 = 2;
pub const AUTHENTIC_REMOTE: u32 = 3;
/// Acct-Terminate-Cause (RFC 2866 5.10)
pub const TERM_USER_REQUEST: u32 = 1;
pub const TERM_LOST_CARRIER: u32 = 2;
pub const TERM_LOST_SERVICE: u32 = 3;
pub const TERM_IDLE_TIMEOUT: u32 = 4;
pub const TERM_SESSION_TIMEOUT: u32 = 5;
pub const TERM_ADMIN_RESET: u32 = 6;
pub const TERM_ADMIN_REBOOT: u32 = 7;
pub const TERM_NAS_ERROR: u32 = 9;
pub const TERM_NAS_REQUEST: u32 = 10;
pub const TERM_NAS_REBOOT: u32 = 11;

/// the name of an attribute type
pub fn attr_name(t: u8) -> String {
    String::from(match t {
        A_USER_NAME => "User-Name",
        A_USER_PASSWORD => "User-Password",
        A_CHAP_PASSWORD => "CHAP-Password",
        A_NAS_IP_ADDRESS => "NAS-IP-Address",
        A_NAS_PORT => "NAS-Port",
        A_SERVICE_TYPE => "Service-Type",
        A_FRAMED_PROTOCOL => "Framed-Protocol",
        A_FRAMED_IP_ADDRESS => "Framed-IP-Address",
        A_FRAMED_IP_NETMASK => "Framed-IP-Netmask",
        A_FRAMED_ROUTING => "Framed-Routing",
        A_FILTER_ID => "Filter-Id",
        A_FRAMED_MTU => "Framed-MTU",
        A_FRAMED_COMPRESSION => "Framed-Compression",
        A_LOGIN_IP_HOST => "Login-IP-Host",
        A_LOGIN_SERVICE => "Login-Service",
        A_LOGIN_TCP_PORT => "Login-TCP-Port",
        A_REPLY_MESSAGE => "Reply-Message",
        A_CALLBACK_NUMBER => "Callback-Number",
        A_CALLBACK_ID => "Callback-Id",
        A_FRAMED_ROUTE => "Framed-Route",
        A_FRAMED_IPX_NETWORK => "Framed-IPX-Network",
        A_STATE => "State",
        A_CLASS => "Class",
        A_VENDOR_SPECIFIC => "Vendor-Specific",
        A_SESSION_TIMEOUT => "Session-Timeout",
        A_IDLE_TIMEOUT => "Idle-Timeout",
        A_TERMINATION_ACTION => "Termination-Action",
        A_CALLED_STATION_ID => "Called-Station-Id",
        A_CALLING_STATION_ID => "Calling-Station-Id",
        A_NAS_IDENTIFIER => "NAS-Identifier",
        A_PROXY_STATE => "Proxy-State",
        A_LOGIN_LAT_SERVICE => "Login-LAT-Service",
        A_LOGIN_LAT_NODE => "Login-LAT-Node",
        A_LOGIN_LAT_GROUP => "Login-LAT-Group",
        A_FRAMED_APPLETALK_LINK => "Framed-AppleTalk-Link",
        A_FRAMED_APPLETALK_NETWORK => "Framed-AppleTalk-Network",
        A_FRAMED_APPLETALK_ZONE => "Framed-AppleTalk-Zone",
        A_ACCT_STATUS_TYPE => "Acct-Status-Type",
        A_ACCT_DELAY_TIME => "Acct-Delay-Time",
        A_ACCT_INPUT_OCTETS => "Acct-Input-Octets",
        A_ACCT_OUTPUT_OCTETS => "Acct-Output-Octets",
        A_ACCT_SESSION_ID => "Acct-Session-Id",
        A_ACCT_AUTHENTIC => "Acct-Authentic",
        A_ACCT_SESSION_TIME => "Acct-Session-Time",
        A_ACCT_INPUT_PACKETS => "Acct-Input-Packets",
        A_ACCT_OUTPUT_PACKETS => "Acct-Output-Packets",
        A_ACCT_TERMINATE_CAUSE => "Acct-Terminate-Cause",
        A_ACCT_MULTI_SESSION_ID => "Acct-Multi-Session-Id",
        A_ACCT_LINK_COUNT => "Acct-Link-Count",
        A_CHAP_CHALLENGE => "CHAP-Challenge",
        A_NAS_PORT_TYPE => "NAS-Port-Type",
        A_PORT_LIMIT => "Port-Limit",
        A_LOGIN_LAT_PORT => "Login-LAT-Port",
        A_EAP_MESSAGE => "EAP-Message",
        A_MESSAGE_AUTHENTICATOR => "Message-Authenticator",
        _ => return format!("Attribute {}", t),
    })
}

/// what the value of an attribute is (RFC 2865 5: text, string, address, integer, time)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// UTF-8 text
    Text,
    /// bytes
    String,
    /// an IPv4 address (4 bytes)
    Address,
    /// a 32-bit number (some with named values)
    Integer,
    /// seconds since 1970 (32 bits)
    Time,
    /// User-Password: hidden with the shared secret
    Hidden,
    /// a vendor's number and its data
    Vendor,
    /// unknown types: bytes
    Unknown,
}

/// the kind of an attribute's value
pub fn attr_kind(t: u8) -> Kind {
    match t {
        A_USER_NAME | A_FILTER_ID | A_REPLY_MESSAGE | A_CALLBACK_NUMBER | A_CALLBACK_ID | A_FRAMED_ROUTE | A_ACCT_SESSION_ID | A_ACCT_MULTI_SESSION_ID => Kind::Text,
        A_USER_PASSWORD => Kind::Hidden,
        A_NAS_IP_ADDRESS | A_FRAMED_IP_ADDRESS | A_FRAMED_IP_NETMASK | A_LOGIN_IP_HOST => Kind::Address,
        A_NAS_PORT | A_SERVICE_TYPE | A_FRAMED_PROTOCOL | A_FRAMED_ROUTING | A_FRAMED_MTU | A_FRAMED_COMPRESSION | A_LOGIN_SERVICE | A_LOGIN_TCP_PORT | A_FRAMED_IPX_NETWORK | A_SESSION_TIMEOUT | A_IDLE_TIMEOUT
        | A_TERMINATION_ACTION | A_FRAMED_APPLETALK_LINK | A_FRAMED_APPLETALK_NETWORK | A_ACCT_STATUS_TYPE | A_ACCT_DELAY_TIME | A_ACCT_INPUT_OCTETS | A_ACCT_OUTPUT_OCTETS | A_ACCT_AUTHENTIC | A_ACCT_SESSION_TIME
        | A_ACCT_INPUT_PACKETS | A_ACCT_OUTPUT_PACKETS | A_ACCT_TERMINATE_CAUSE | A_ACCT_LINK_COUNT | A_NAS_PORT_TYPE | A_PORT_LIMIT => Kind::Integer,
        A_CHAP_PASSWORD | A_STATE | A_CLASS | A_CALLED_STATION_ID | A_CALLING_STATION_ID | A_NAS_IDENTIFIER | A_PROXY_STATE | A_LOGIN_LAT_SERVICE | A_LOGIN_LAT_NODE | A_LOGIN_LAT_GROUP | A_FRAMED_APPLETALK_ZONE
        | A_CHAP_CHALLENGE | A_LOGIN_LAT_PORT | A_EAP_MESSAGE | A_MESSAGE_AUTHENTICATOR => Kind::String,
        A_VENDOR_SPECIFIC => Kind::Vendor,
        _ => Kind::Unknown,
    }
}

/// the name of a value of an integer attribute that has named values
pub fn value_name(t: u8, v: u32) -> Option<&'static str> {
    Some(match (t, v) {
        (A_SERVICE_TYPE, 1) => "Login-User",
        (A_SERVICE_TYPE, 2) => "Framed-User",
        (A_SERVICE_TYPE, 3) => "Callback-Login-User",
        (A_SERVICE_TYPE, 4) => "Callback-Framed-User",
        (A_SERVICE_TYPE, 5) => "Outbound-User",
        (A_SERVICE_TYPE, 6) => "Administrative-User",
        (A_SERVICE_TYPE, 7) => "NAS-Prompt-User",
        (A_SERVICE_TYPE, 8) => "Authenticate-Only",
        (A_SERVICE_TYPE, 9) => "Callback-NAS-Prompt",
        (A_SERVICE_TYPE, 10) => "Call-Check",
        (A_SERVICE_TYPE, 11) => "Callback-Administrative",
        (A_FRAMED_PROTOCOL, 1) => "PPP",
        (A_FRAMED_PROTOCOL, 2) => "SLIP",
        (A_FRAMED_PROTOCOL, 3) => "ARAP",
        (A_FRAMED_PROTOCOL, 4) => "Gandalf-SLML",
        (A_FRAMED_PROTOCOL, 5) => "Xylogics-IPX-SLIP",
        (A_FRAMED_PROTOCOL, 6) => "X.75-Synchronous",
        (A_LOGIN_SERVICE, 0) => "Telnet",
        (A_LOGIN_SERVICE, 1) => "Rlogin",
        (A_LOGIN_SERVICE, 2) => "TCP-Clear",
        (A_LOGIN_SERVICE, 3) => "PortMaster",
        (A_LOGIN_SERVICE, 4) => "LAT",
        (A_LOGIN_SERVICE, 5) => "X25-PAD",
        (A_LOGIN_SERVICE, 6) => "X25-T3POS",
        (A_LOGIN_SERVICE, 8) => "TCP-Clear-Quiet",
        (A_TERMINATION_ACTION, 0) => "Default",
        (A_TERMINATION_ACTION, 1) => "RADIUS-Request",
        (A_NAS_PORT_TYPE, 0) => "Async",
        (A_NAS_PORT_TYPE, 1) => "Sync",
        (A_NAS_PORT_TYPE, 2) => "ISDN-Sync",
        (A_NAS_PORT_TYPE, 3) => "ISDN-Async-V.120",
        (A_NAS_PORT_TYPE, 4) => "ISDN-Async-V.110",
        (A_NAS_PORT_TYPE, 5) => "Virtual",
        (A_NAS_PORT_TYPE, 6) => "PIAFS",
        (A_NAS_PORT_TYPE, 7) => "HDLC-Clear-Channel",
        (A_NAS_PORT_TYPE, 8) => "X.25",
        (A_NAS_PORT_TYPE, 9) => "X.75",
        (A_NAS_PORT_TYPE, 10) => "G.3-Fax",
        (A_NAS_PORT_TYPE, 11) => "SDSL",
        (A_NAS_PORT_TYPE, 12) => "ADSL-CAP",
        (A_NAS_PORT_TYPE, 13) => "ADSL-DMT",
        (A_NAS_PORT_TYPE, 14) => "IDSL",
        (A_NAS_PORT_TYPE, 15) => "Ethernet",
        (A_NAS_PORT_TYPE, 16) => "xDSL",
        (A_NAS_PORT_TYPE, 17) => "Cable",
        (A_NAS_PORT_TYPE, 18) => "Wireless-Other",
        (A_NAS_PORT_TYPE, 19) => "Wireless-802.11",
        (A_ACCT_STATUS_TYPE, 1) => "Start",
        (A_ACCT_STATUS_TYPE, 2) => "Stop",
        (A_ACCT_STATUS_TYPE, 3) => "Interim-Update",
        (A_ACCT_STATUS_TYPE, 7) => "Accounting-On",
        (A_ACCT_STATUS_TYPE, 8) => "Accounting-Off",
        (A_ACCT_AUTHENTIC, 1) => "RADIUS",
        (A_ACCT_AUTHENTIC, 2) => "Local",
        (A_ACCT_AUTHENTIC, 3) => "Remote",
        (A_ACCT_TERMINATE_CAUSE, 1) => "User-Request",
        (A_ACCT_TERMINATE_CAUSE, 2) => "Lost-Carrier",
        (A_ACCT_TERMINATE_CAUSE, 3) => "Lost-Service",
        (A_ACCT_TERMINATE_CAUSE, 4) => "Idle-Timeout",
        (A_ACCT_TERMINATE_CAUSE, 5) => "Session-Timeout",
        (A_ACCT_TERMINATE_CAUSE, 6) => "Admin-Reset",
        (A_ACCT_TERMINATE_CAUSE, 7) => "Admin-Reboot",
        (A_ACCT_TERMINATE_CAUSE, 8) => "Port-Error",
        (A_ACCT_TERMINATE_CAUSE, 9) => "NAS-Error",
        (A_ACCT_TERMINATE_CAUSE, 10) => "NAS-Request",
        (A_ACCT_TERMINATE_CAUSE, 11) => "NAS-Reboot",
        (A_ACCT_TERMINATE_CAUSE, 12) => "Port-Unneeded",
        (A_ACCT_TERMINATE_CAUSE, 13) => "Port-Preempted",
        (A_ACCT_TERMINATE_CAUSE, 14) => "Port-Suspended",
        (A_ACCT_TERMINATE_CAUSE, 15) => "Service-Unavailable",
        (A_ACCT_TERMINATE_CAUSE, 16) => "Callback",
        (A_ACCT_TERMINATE_CAUSE, 17) => "User-Error",
        (A_ACCT_TERMINATE_CAUSE, 18) => "Host-Request",
        _ => return None,
    })
}

/// why bytes are no RADIUS packet (a receiver discards them silently)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bad {
    /// shorter than the header, or than the Length field says
    Short,
    /// a Length field below 20 or above 4096
    Length(u16),
    /// an attribute with a length below 2, or one that does not end inside the packet
    Attr(usize),
}

/// an attribute
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attr {
    pub typ: u8,
    pub value: Vec<u8>,
}

impl Attr {
    pub fn bytes(typ: u8, value: &[u8]) -> Attr {
        Attr { typ, value: value[..value.len().min(VALUE_MAX)].to_vec() }
    }
    pub fn text(typ: u8, s: &str) -> Attr {
        Attr::bytes(typ, s.as_bytes())
    }
    pub fn int(typ: u8, v: u32) -> Attr {
        Attr { typ, value: v.to_be_bytes().to_vec() }
    }
    pub fn addr(typ: u8, a: Ipv4) -> Attr {
        Attr { typ, value: a.0.to_vec() }
    }
    /// a Vendor-Specific attribute: the vendor's SMI number and its data, kept as bytes
    pub fn vsa(vendor: u32, data: &[u8]) -> Attr {
        let mut v = vendor.to_be_bytes().to_vec();
        v.extend_from_slice(&data[..data.len().min(VALUE_MAX - 4)]);
        Attr { typ: A_VENDOR_SPECIFIC, value: v }
    }
    /// a Message-Authenticator to be filled in (set_message_auth)
    pub fn message_auth() -> Attr {
        Attr { typ: A_MESSAGE_AUTHENTICATOR, value: alloc::vec![0; 16] }
    }
    /// the vendor and its data of a Vendor-Specific attribute
    pub fn vendor(&self) -> Option<(u32, &[u8])> {
        if self.typ != A_VENDOR_SPECIFIC || self.value.len() < 4 {
            return None;
        }
        Some((be32(&self.value, 0), &self.value[4..]))
    }
    /// the value as a number (4 bytes)
    pub fn as_int(&self) -> Option<u32> {
        (self.value.len() == 4).then(|| be32(&self.value, 0))
    }
    /// the value as an address (4 bytes)
    pub fn as_addr(&self) -> Option<Ipv4> {
        (self.value.len() == 4).then(|| Ipv4([self.value[0], self.value[1], self.value[2], self.value[3]]))
    }
    /// the value as text (bytes that are no UTF-8 are replaced)
    pub fn as_text(&self) -> String {
        String::from_utf8_lossy(&self.value).into_owned()
    }
}

/// a RADIUS packet, checked once
#[derive(Clone, Copy, Debug)]
pub struct RadiusView<'a> {
    b: &'a [u8],
}

impl<'a> RadiusView<'a> {
    /// the packet at the start of `b` (bytes after its Length are padding)
    pub fn parse(b: &'a [u8]) -> Result<RadiusView<'a>, Bad> {
        if b.len() < HDR {
            return Err(Bad::Short);
        }
        let len = be16(b, 2);
        if (len as usize) < HDR || len as usize > MAX {
            return Err(Bad::Length(len));
        }
        if b.len() < len as usize {
            return Err(Bad::Short);
        }
        let b = &b[..len as usize];
        let mut i = HDR;
        while i < b.len() {
            if i + 2 > b.len() || b[i + 1] < 2 || i + b[i + 1] as usize > b.len() {
                return Err(Bad::Attr(i));
            }
            i += b[i + 1] as usize;
        }
        Ok(RadiusView { b })
    }
    pub fn code(&self) -> u8 {
        self.b[0]
    }
    pub fn id(&self) -> u8 {
        self.b[1]
    }
    pub fn length(&self) -> u16 {
        be16(self.b, 2)
    }
    pub fn authenticator(&self) -> [u8; 16] {
        let mut a = [0u8; 16];
        a.copy_from_slice(&self.b[4..20]);
        a
    }
    /// the packet's bytes (up to Length)
    pub fn bytes(&self) -> &'a [u8] {
        self.b
    }
    /// every attribute: its offset in the packet, its type and its value
    pub fn attrs(&self) -> Vec<(usize, u8, &'a [u8])> {
        let mut out = Vec::new();
        let mut i = HDR;
        while i + 2 <= self.b.len() {
            let l = self.b[i + 1] as usize;
            out.push((i, self.b[i], &self.b[i + 2..i + l]));
            i += l;
        }
        out
    }
}

/// a RADIUS packet as a record
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Packet {
    pub code: u8,
    pub id: u8,
    /// the Request or Response Authenticator
    pub auth: [u8; 16],
    pub attrs: Vec<Attr>,
}

impl Packet {
    pub fn new(code: u8, id: u8, auth: [u8; 16]) -> Packet {
        Packet { code, id, auth, attrs: Vec::new() }
    }
    pub fn read(v: &RadiusView) -> Packet {
        Packet { code: v.code(), id: v.id(), auth: v.authenticator(), attrs: v.attrs().into_iter().map(|(_, t, val)| Attr { typ: t, value: val.to_vec() }).collect() }
    }
    pub fn parse(b: &[u8]) -> Result<Packet, Bad> {
        RadiusView::parse(b).map(|v| Packet::read(&v))
    }
    /// the bytes (an attribute value longer than 253 bytes is cut; the whole is at most 4096)
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut b = alloc::vec![self.code, self.id, 0, 0];
        b.extend_from_slice(&self.auth);
        for a in &self.attrs {
            let v = &a.value[..a.value.len().min(VALUE_MAX)];
            if b.len() + 2 + v.len() > MAX {
                break;
            }
            b.push(a.typ);
            b.push(v.len() as u8 + 2);
            b.extend_from_slice(v);
        }
        let n = b.len() as u16;
        put16(&mut b, 2, n);
        b
    }
    pub fn push(&mut self, a: Attr) -> &mut Packet {
        self.attrs.push(a);
        self
    }
    /// the first attribute of a type
    pub fn attr(&self, t: u8) -> Option<&Attr> {
        self.attrs.iter().find(|a| a.typ == t)
    }
    /// every attribute of a type
    pub fn all(&self, t: u8) -> Vec<&Attr> {
        self.attrs.iter().filter(|a| a.typ == t).collect()
    }
    pub fn text(&self, t: u8) -> Option<String> {
        self.attr(t).map(Attr::as_text)
    }
    pub fn int(&self, t: u8) -> Option<u32> {
        self.attr(t).and_then(Attr::as_int)
    }
    pub fn addr(&self, t: u8) -> Option<Ipv4> {
        self.attr(t).and_then(Attr::as_addr)
    }
}

// ---------------------------------------------------------------- User-Password (RFC 2865 5.2)

/// User-Password hidden: padded with zeros to a multiple of 16 (at most 128 bytes), each block
/// XORed with MD5(secret + the previous block of the result; the Request Authenticator first)
pub fn hide_password(pw: &[u8], secret: &[u8], auth: &[u8; 16], md5: Md5Fn) -> Vec<u8> {
    let n = pw.len().clamp(1, 128).div_ceil(16) * 16;
    let mut p = alloc::vec![0u8; n];
    let k = pw.len().min(128);
    p[..k].copy_from_slice(&pw[..k]);
    let mut prev: [u8; 16] = *auth;
    for chunk in p.chunks_mut(16) {
        let b = md5(&[secret, &prev]);
        for (x, y) in chunk.iter_mut().zip(b) {
            *x ^= y;
        }
        prev.copy_from_slice(chunk);
    }
    p
}

/// the password of a hidden User-Password (its padding zeros removed); None when the length is
/// no multiple of 16 between 16 and 128
pub fn reveal_password(hidden: &[u8], secret: &[u8], auth: &[u8; 16], md5: Md5Fn) -> Option<Vec<u8>> {
    if hidden.is_empty() || hidden.len() % 16 != 0 || hidden.len() > 128 {
        return None;
    }
    let mut out = Vec::with_capacity(hidden.len());
    let mut prev: [u8; 16] = *auth;
    for chunk in hidden.chunks(16) {
        let b = md5(&[secret, &prev]);
        out.extend(chunk.iter().zip(b).map(|(x, y)| x ^ y));
        prev.copy_from_slice(chunk);
    }
    while out.last() == Some(&0) {
        out.pop();
    }
    Some(out)
}

// ---------------------------------------------------------------- authenticators

/// MD5(Code + Identifier + Length + `auth` + Attributes + secret) over a written packet
fn auth_over(b: &[u8], auth: &[u8; 16], secret: &[u8], md5: Md5Fn) -> [u8; 16] {
    if b.len() < HDR {
        return [0; 16];
    }
    md5(&[&b[..4], auth, &b[HDR..], secret])
}

/// the Response Authenticator of a written answer to a request with `req_auth`
pub fn response_auth(b: &[u8], req_auth: &[u8; 16], secret: &[u8], md5: Md5Fn) -> [u8; 16] {
    auth_over(b, req_auth, secret, md5)
}

/// writes the Response Authenticator into an answer (after a Message-Authenticator)
pub fn set_response_auth(b: &mut [u8], req_auth: &[u8; 16], secret: &[u8], md5: Md5Fn) {
    if b.len() >= HDR {
        let a = response_auth(b, req_auth, secret, md5);
        b[4..20].copy_from_slice(&a);
    }
}

/// the answer's Response Authenticator is right for the request
pub fn check_response_auth(b: &[u8], req_auth: &[u8; 16], secret: &[u8], md5: Md5Fn) -> bool {
    b.len() >= HDR && b[4..20] == response_auth(b, req_auth, secret, md5)
}

/// the Request Authenticator of an Accounting-Request (RFC 2866 3: over 16 zero bytes)
pub fn accounting_auth(b: &[u8], secret: &[u8], md5: Md5Fn) -> [u8; 16] {
    auth_over(b, &[0; 16], secret, md5)
}

/// writes the Request Authenticator into an Accounting-Request
pub fn set_accounting_auth(b: &mut [u8], secret: &[u8], md5: Md5Fn) {
    if b.len() >= HDR {
        let a = accounting_auth(b, secret, md5);
        b[4..20].copy_from_slice(&a);
    }
}

/// an Accounting-Request's Request Authenticator fits the secret
pub fn check_accounting_auth(b: &[u8], secret: &[u8], md5: Md5Fn) -> bool {
    b.len() >= HDR && b[4..20] == accounting_auth(b, secret, md5)
}

/// where the Message-Authenticator's value sits in a written packet
fn message_auth_at(b: &[u8]) -> Option<usize> {
    let v = RadiusView::parse(b).ok()?;
    v.attrs().into_iter().find(|a| a.1 == A_MESSAGE_AUTHENTICATOR && a.2.len() == 16).map(|a| a.0 + 2)
}

/// HMAC-MD5 with the secret over the packet with `auth` in its authenticator field and the
/// Message-Authenticator counted as 16 zero bytes (RFC 3579 3.2)
pub fn message_auth(b: &[u8], auth: &[u8; 16], secret: &[u8], hmac: HmacMd5Fn) -> [u8; 16] {
    let mut c = b.to_vec();
    if c.len() >= HDR {
        c[4..20].copy_from_slice(auth);
    }
    if let Some(at) = message_auth_at(&c) {
        for x in &mut c[at..at + 16] {
            *x = 0;
        }
    }
    hmac(secret, &c)
}

/// fills in the Message-Authenticator of a written packet (`auth`: None for a request, which
/// covers its own authenticator; the request's for an answer); false when there is none
pub fn set_message_auth(b: &mut [u8], auth: Option<&[u8; 16]>, secret: &[u8], hmac: HmacMd5Fn) -> bool {
    let Some(at) = message_auth_at(b) else {
        return false;
    };
    let own = {
        let mut a = [0u8; 16];
        a.copy_from_slice(&b[4..20]);
        a
    };
    let m = message_auth(b, auth.unwrap_or(&own), secret, hmac);
    b[at..at + 16].copy_from_slice(&m);
    true
}

/// the Message-Authenticator of a packet fits the secret (None: the packet has none)
pub fn check_message_auth(b: &[u8], auth: Option<&[u8; 16]>, secret: &[u8], hmac: HmacMd5Fn) -> Option<bool> {
    let at = message_auth_at(b)?;
    let mut own = [0u8; 16];
    own.copy_from_slice(&b[4..20]);
    let m = message_auth(b, auth.unwrap_or(&own), secret, hmac);
    Some(b[at..at + 16] == m)
}

/// the value of an attribute as a dissector shows it (a hidden password stays hidden)
pub fn value_text(t: u8, v: &[u8]) -> String {
    match attr_kind(t) {
        Kind::Text => format!("\"{}\"", String::from_utf8_lossy(v)),
        Kind::Hidden => format!("(hidden, {} bytes)", v.len()),
        Kind::Address if v.len() == 4 => format!("{}", Ipv4([v[0], v[1], v[2], v[3]])),
        Kind::Integer if v.len() == 4 => {
            let n = be32(v, 0);
            match value_name(t, n) {
                Some(name) => format!("{} ({})", name, n),
                None => format!("{}", n),
            }
        }
        Kind::Time if v.len() == 4 => format!("{}", be32(v, 0)),
        Kind::Vendor if v.len() >= 4 => format!("vendor {}, {} bytes", be32(v, 0), v.len() - 4),
        _ => {
            let mut s = String::new();
            for b in v.iter().take(32) {
                s.push_str(&format!("{:02x}", b));
            }
            if v.len() > 32 {
                s.push_str("...");
            }
            s
        }
    }
}
