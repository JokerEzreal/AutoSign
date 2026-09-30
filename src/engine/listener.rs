//! Firestore 实时监听:用 gRPC `Listen` 订阅公开的 ongoingClasses,一有新课解锁就唤醒轮询器,
//! 把「老师解锁 → 我们签到」的延迟从最长一个轮询周期压到亚秒级。
//!
//! 只做触发,不做判断:课程数据仍由 poller 经 REST 重新拉取、去重、计费;监听断线时自动退避重连,
//! 期间 5s 轮询照常兜底,所以监听的任何故障都不会影响正确性。

use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use tonic::codec::ProstCodec;
use tonic::codegen::http::uri::PathAndQuery;
use tonic::metadata::MetadataValue;
use tonic::transport::{Channel, ClientTlsConfig};
use tonic::Request;

use crate::engine::instatt::FIREBASE_PROJECT_ID;
use crate::engine::worker::Engine;

const FIRESTORE_GRPC: &str = "https://firestore.googleapis.com";
const LISTEN_PATH: &str = "/google.firestore.v1.Firestore/Listen";
const TARGET_ID: i32 = 1;

/// google.firestore.v1 的最小子集(手写 prost 定义,只含 Listen 用到的字段;解码时未知字段会被跳过)。
pub mod proto {
    use std::collections::HashMap;

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ListenRequest {
        #[prost(string, tag = "1")]
        pub database: String,
        #[prost(oneof = "listen_request::TargetChange", tags = "2, 3")]
        pub target_change: Option<listen_request::TargetChange>,
    }
    pub mod listen_request {
        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum TargetChange {
            #[prost(message, tag = "2")]
            AddTarget(super::Target),
            #[prost(int32, tag = "3")]
            RemoveTarget(i32),
        }
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Target {
        #[prost(oneof = "target::TargetType", tags = "2, 3")]
        pub target_type: Option<target::TargetType>,
        #[prost(int32, tag = "5")]
        pub target_id: i32,
        #[prost(bool, tag = "6")]
        pub once: bool,
    }
    pub mod target {
        #[derive(Clone, PartialEq, prost::Message)]
        pub struct QueryTarget {
            #[prost(string, tag = "1")]
            pub parent: String,
            #[prost(message, optional, tag = "2")]
            pub structured_query: Option<super::StructuredQuery>,
        }
        #[derive(Clone, PartialEq, prost::Message)]
        pub struct DocumentsTarget {
            #[prost(string, repeated, tag = "2")]
            pub documents: Vec<String>,
        }
        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum TargetType {
            #[prost(message, tag = "2")]
            Query(QueryTarget),
            #[prost(message, tag = "3")]
            Documents(DocumentsTarget),
        }
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct StructuredQuery {
        #[prost(message, repeated, tag = "2")]
        pub from: Vec<CollectionSelector>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct CollectionSelector {
        #[prost(string, tag = "2")]
        pub collection_id: String,
        #[prost(bool, tag = "3")]
        pub all_descendants: bool,
    }

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ListenResponse {
        #[prost(oneof = "listen_response::ResponseType", tags = "2, 3, 4, 5, 6")]
        pub response_type: Option<listen_response::ResponseType>,
    }
    pub mod listen_response {
        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum ResponseType {
            #[prost(message, tag = "2")]
            TargetChange(super::TargetChange),
            #[prost(message, tag = "3")]
            DocumentChange(super::DocumentChange),
            #[prost(message, tag = "4")]
            DocumentDelete(super::DocumentDelete),
            #[prost(message, tag = "5")]
            Filter(super::ExistenceFilter),
            #[prost(message, tag = "6")]
            DocumentRemove(super::DocumentRemove),
        }
    }

    /// TargetChange.target_change_type 取值。
    pub const TC_NO_CHANGE: i32 = 0;
    pub const TC_ADD: i32 = 1;
    pub const TC_REMOVE: i32 = 2;
    pub const TC_CURRENT: i32 = 3;
    pub const TC_RESET: i32 = 4;

    #[derive(Clone, PartialEq, prost::Message)]
    pub struct TargetChange {
        #[prost(int32, tag = "1")]
        pub target_change_type: i32,
        #[prost(int32, repeated, tag = "2")]
        pub target_ids: Vec<i32>,
        #[prost(message, optional, tag = "3")]
        pub cause: Option<Status>,
        #[prost(bytes = "vec", tag = "4")]
        pub resume_token: Vec<u8>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Status {
        #[prost(int32, tag = "1")]
        pub code: i32,
        #[prost(string, tag = "2")]
        pub message: String,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct DocumentChange {
        #[prost(message, optional, tag = "1")]
        pub document: Option<Document>,
        #[prost(int32, repeated, tag = "5")]
        pub target_ids: Vec<i32>,
        #[prost(int32, repeated, tag = "6")]
        pub removed_target_ids: Vec<i32>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct DocumentDelete {
        #[prost(string, tag = "1")]
        pub document: String,
        #[prost(int32, repeated, tag = "6")]
        pub removed_target_ids: Vec<i32>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct DocumentRemove {
        #[prost(string, tag = "1")]
        pub document: String,
        #[prost(int32, repeated, tag = "2")]
        pub removed_target_ids: Vec<i32>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct ExistenceFilter {
        #[prost(int32, tag = "1")]
        pub target_id: i32,
        #[prost(int32, tag = "2")]
        pub count: i32,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Document {
        #[prost(string, tag = "1")]
        pub name: String,
        #[prost(map = "string, message", tag = "2")]
        pub fields: HashMap<String, Value>,
        /// 服务端最近一次写入时间(google.protobuf.Timestamp),用于计算推送延迟。
        #[prost(message, optional, tag = "4")]
        pub update_time: Option<Timestamp>,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Timestamp {
        #[prost(int64, tag = "1")]
        pub seconds: i64,
        #[prost(int32, tag = "2")]
        pub nanos: i32,
    }
    #[derive(Clone, PartialEq, prost::Message)]
    pub struct Value {
        #[prost(oneof = "value::ValueType", tags = "1, 2, 17")]
        pub value_type: Option<value::ValueType>,
    }
    pub mod value {
        #[derive(Clone, PartialEq, prost::Oneof)]
        pub enum ValueType {
            #[prost(bool, tag = "1")]
            BooleanValue(bool),
            #[prost(int64, tag = "2")]
            IntegerValue(i64),
            #[prost(string, tag = "17")]
            StringValue(String),
        }
    }

    impl Document {
        /// 文档 id(name 最后一段,如 COMP4082_AUM_26-27_20260930_1400_F1A24)。
        pub fn id(&self) -> &str {
            self.name.rsplit('/').next().unwrap_or(&self.name)
        }
        /// 整数字段,缺失为 0。
        pub fn int(&self, key: &str) -> i64 {
            match self.fields.get(key).and_then(|v| v.value_type.as_ref()) {
                Some(value::ValueType::IntegerValue(i)) => *i,
                _ => 0,
            }
        }
        /// 服务端写入到现在的延迟(毫秒);无 update_time 时为 -1。
        pub fn age_ms(&self) -> i64 {
            let Some(t) = &self.update_time else { return -1 };
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            now - (t.seconds * 1000 + t.nanos as i64 / 1_000_000)
        }
    }
}

use proto::listen_response::ResponseType;

fn database_path() -> String {
    format!("projects/{FIREBASE_PROJECT_ID}/databases/(default)")
}

/// 订阅 ongoingClasses 集合的 AddTarget 请求。
fn add_target_request() -> proto::ListenRequest {
    let db = database_path();
    proto::ListenRequest {
        database: db.clone(),
        target_change: Some(proto::listen_request::TargetChange::AddTarget(proto::Target {
            target_type: Some(proto::target::TargetType::Query(proto::target::QueryTarget {
                parent: format!("{db}/documents"),
                structured_query: Some(proto::StructuredQuery {
                    from: vec![proto::CollectionSelector {
                        collection_id: "ongoingClasses".into(),
                        all_descendants: false,
                    }],
                }),
            })),
            target_id: TARGET_ID,
            once: false,
        })),
    }
}

async fn connect() -> anyhow::Result<Channel> {
    let tls = ClientTlsConfig::new().with_webpki_roots();
    Ok(Channel::from_static(FIRESTORE_GRPC)
        .tls_config(tls)?
        .connect_timeout(Duration::from_secs(15))
        .http2_keep_alive_interval(Duration::from_secs(30))
        .keep_alive_timeout(Duration::from_secs(10))
        .keep_alive_while_idle(true)
        .connect()
        .await?)
}

/// 建立一次监听流并消费到断开。返回 Ok 表示上游正常关闭,Err 表示出错;两者都由调用方重连。
async fn listen_once(engine: &Engine) -> anyhow::Result<()> {
    let channel = connect().await?;
    let mut grpc = tonic::client::Grpc::new(channel);
    grpc.ready().await?;

    // 请求流:先发 AddTarget,之后保持 tx 存活让流不结束
    let (tx, rx) = mpsc::channel::<proto::ListenRequest>(4);
    tx.send(add_target_request()).await?;
    let mut req = Request::new(ReceiverStream::new(rx));
    let db = database_path();
    req.metadata_mut()
        .insert("google-cloud-resource-prefix", MetadataValue::try_from(db.as_str())?);
    req.metadata_mut()
        .insert("x-goog-request-params", MetadataValue::try_from(format!("database={db}").as_str())?);

    let codec: ProstCodec<proto::ListenRequest, proto::ListenResponse> = ProstCodec::default();
    let mut stream = grpc
        .streaming(req, PathAndQuery::from_static(LISTEN_PATH), codec)
        .await?
        .into_inner();

    let mut seen: HashSet<String> = HashSet::new();
    let mut current = false; // 收到 CURRENT 之前的 DocumentChange 属于初始快照,不算「新解锁」
    while let Some(msg) = stream.message().await? {
        match msg.response_type {
            Some(ResponseType::TargetChange(tc)) => match tc.target_change_type {
                proto::TC_CURRENT if !current => {
                    current = true;
                    tracing::info!("Firestore 实时监听就绪,当前 {} 门课已解锁", seen.len());
                    engine.wake.notify_one();
                }
                proto::TC_RESET => {
                    // 服务端要求重放:清空已知集合,后续 CURRENT 前的变更再次视为快照
                    seen.clear();
                    current = false;
                }
                proto::TC_REMOVE => {
                    let cause = tc.cause.map(|s| format!("{} {}", s.code, s.message)).unwrap_or_default();
                    anyhow::bail!("目标被移除: {cause}");
                }
                _ => {}
            },
            Some(ResponseType::DocumentChange(dc)) => {
                if let Some(doc) = dc.document {
                    if dc.removed_target_ids.contains(&TARGET_ID) {
                        seen.remove(&doc.name);
                    } else if seen.insert(doc.name.clone()) && current {
                        tracing::info!(
                            "实时:新解锁 {}(unlock={},服务端写入→收到 {} ms),立即触发签到",
                            doc.id(),
                            doc.int("unlockDateTimeStamp"),
                            doc.age_ms()
                        );
                        engine.wake.notify_one();
                    }
                }
            }
            Some(ResponseType::DocumentDelete(d)) => {
                seen.remove(&d.document);
            }
            Some(ResponseType::DocumentRemove(r)) => {
                seen.remove(&r.document);
            }
            Some(ResponseType::Filter(_)) | None => {}
        }
    }
    drop(tx);
    Ok(())
}

/// 常驻监听循环:断开后指数退避重连(1s → 60s),连稳超过 1 分钟即重置退避。
pub async fn run_listener(engine: Arc<Engine>) {
    let mut backoff = Duration::from_secs(1);
    loop {
        let started = Instant::now();
        match listen_once(&engine).await {
            Ok(()) => tracing::info!("Firestore 监听流结束,{backoff:?} 后重连"),
            Err(e) => tracing::warn!("Firestore 监听中断: {e:#},{backoff:?} 后重连"),
        }
        if started.elapsed() > Duration::from_secs(60) {
            backoff = Duration::from_secs(1);
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(Duration::from_secs(60));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

    #[test]
    fn add_target_request_encodes_and_round_trips() {
        let req = add_target_request();
        let bytes = req.encode_to_vec();
        let back = proto::ListenRequest::decode(bytes.as_slice()).unwrap();
        assert_eq!(back, req);
        assert_eq!(back.database, format!("projects/{FIREBASE_PROJECT_ID}/databases/(default)"));
        match back.target_change.unwrap() {
            proto::listen_request::TargetChange::AddTarget(t) => {
                assert_eq!(t.target_id, TARGET_ID);
                match t.target_type.unwrap() {
                    proto::target::TargetType::Query(q) => {
                        assert!(q.parent.ends_with("/documents"));
                        assert_eq!(q.structured_query.unwrap().from[0].collection_id, "ongoingClasses");
                    }
                    other => panic!("期望 Query,得到 {other:?}"),
                }
            }
            other => panic!("期望 AddTarget,得到 {other:?}"),
        }
    }

    #[test]
    fn document_change_decodes_fields() {
        // 构造一条 ListenResponse{document_change{document{name, fields{moduleKey, unlockDateTimeStamp}}}}
        let mut fields = std::collections::HashMap::new();
        fields.insert(
            "moduleKey".to_string(),
            proto::Value { value_type: Some(proto::value::ValueType::StringValue("COMP4082_AUM_26-27".into())) },
        );
        fields.insert(
            "unlockDateTimeStamp".to_string(),
            proto::Value { value_type: Some(proto::value::ValueType::IntegerValue(202609301405)) },
        );
        let resp = proto::ListenResponse {
            response_type: Some(ResponseType::DocumentChange(proto::DocumentChange {
                document: Some(proto::Document {
                    name: "projects/p/databases/(default)/documents/ongoingClasses/COMP4082_AUM_26-27_20260930_1400_F1A24".into(),
                    fields,
                    update_time: Some(proto::Timestamp { seconds: 1_700_000_000, nanos: 500_000_000 }),
                }),
                target_ids: vec![TARGET_ID],
                removed_target_ids: vec![],
            })),
        };
        let bytes = resp.encode_to_vec();
        let back = proto::ListenResponse::decode(bytes.as_slice()).unwrap();
        match back.response_type.unwrap() {
            ResponseType::DocumentChange(dc) => {
                let doc = dc.document.unwrap();
                assert_eq!(doc.id(), "COMP4082_AUM_26-27_20260930_1400_F1A24");
                assert_eq!(doc.int("unlockDateTimeStamp"), 202609301405);
                assert_eq!(doc.int("missing"), 0);
                assert_eq!(doc.update_time.as_ref().unwrap().seconds, 1_700_000_000);
                assert!(doc.age_ms() > 0, "写入时间在过去,延迟应为正");
                assert_eq!(proto::Document::default().age_ms(), -1);
            }
            other => panic!("期望 DocumentChange,得到 {other:?}"),
        }
    }
}
