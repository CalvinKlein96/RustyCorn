//! OPC (operation code) constants of the ÄKTAprime serial protocol
//! (the instrument's PC protocol). Shared by the link layer and the method store.

pub const OPC_IDENTITY_REQUEST: u16 = 0x1001;
pub const OPC_PROG_NAME: u16 = 0x8001;
pub const OPC_VERSION_STRING: u16 = 0x8002;
pub const OPC_TREND_DATA: u16 = 0x2001;
pub const OPC_BASE_TIME_VOL: u16 = 0x2002;
pub const OPC_RUN_STATE: u16 = 0x2003;
pub const OPC_INJECT: u16 = 0x2004;
pub const OPC_TUBEFEED: u16 = 0x2005;
pub const OPC_STEP_TIME_VOL: u16 = 0x2006;
// Method store (rc-methods `store`): dump at run start, save/load/list/delete/exists.
pub const OPC_METHOD_DUMP_BEGIN: u16 = 0x2007;
pub const OPC_METHOD_DUMP_END: u16 = 0x2008;
pub const OPC_METHOD_SAVE_BEGIN: u16 = 0x2009;
pub const OPC_METHOD_SAVE_END: u16 = 0x2010;
pub const OPC_METHOD_SAVE_ACK: u16 = 0x2011;
pub const OPC_METHOD_LOAD: u16 = 0x2012;
pub const OPC_METHOD_LOAD_BEGIN: u16 = 0x2013;
pub const OPC_METHOD_LOAD_END: u16 = 0x2014;
pub const OPC_METHOD_LIST: u16 = 0x2015;
pub const OPC_METHOD_LIST_REPLY: u16 = 0x2016;
pub const OPC_METHOD_DELETE: u16 = 0x2017;
pub const OPC_METHOD_DELETE_REPLY: u16 = 0x2018;
pub const OPC_METHOD_EXISTS: u16 = 0x2019;
pub const OPC_METHOD_EXISTS_REPLY: u16 = 0x2020;
pub const OPC_DEBUG_MSG: u16 = 0x8003;
pub const OPC_LOG_SINGLE: u16 = 0x8004;
pub const OPC_LOG_BEGIN: u16 = 0x8005;
pub const OPC_LOG_CONT: u16 = 0x8006;
pub const OPC_LOG_END: u16 = 0x8007;
/// a whole line of method text
pub const OPC_METHOD_LINE: u16 = 0x8008;
/// a long line: first part,
pub const OPC_METHOD_LINE_BEGIN: u16 = 0x8009;
/// middle parts,
pub const OPC_METHOD_LINE_CONT: u16 = 0x8010;
/// last part
pub const OPC_METHOD_LINE_END: u16 = 0x8011;
pub const OPC_WATCHDOG: u16 = 0x3001;

/// One OPC message: code and data bytes.
pub type OpcMessage = (u16, Vec<u8>);
