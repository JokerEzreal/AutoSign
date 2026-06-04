//! 签到引擎:上游客户端、token 管理、轮询器、worker、计费。
//! 模块边界清晰,将来可整体拆为独立 binary。

pub mod instatt;
pub mod tokens;
