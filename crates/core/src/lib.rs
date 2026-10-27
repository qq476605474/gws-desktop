//! # gws-core
//!
//! GWS的**全部领域逻辑**。CLI（`gws`）与 Tauri 桌面端共用同一份实现，
//! 保证两边行为永不漂移。
//!
//! ## 三条硬约束
//!
//! 1. **不依赖 Tauri、不依赖 clap** —— 上层怎么用是上层的事。
//! 2. **不直接打印到 stdout** —— 所有操作返回结构化结果（`serde::Serialize`），
//!    由调用方决定渲染成人类可读文本还是 JSON。这是取代旧版「前端正则解析
//!    ANSI 彩色文本」的关键（设计方案 §1.2）。
//! 3. **不 spawn 外部 `gws`** —— 旧版的核心逻辑是 bash 脚本，本版用 Rust 重写。
//!
//! ## 模块地图（按设计方案 §1.1，随实现进度逐个落地）
//!
//! | 模块 | 职责 | 状态 |
//! |---|---|---|
//! | [`error`] | 统一错误类型 | ✅ |
//! | [`name`] | 名称校验（挡住路径穿越与非法字符） | ✅ |
//! | [`paths`] | 跨平台路径（主目录、配置目录） | ✅ |
//! | [`progress`] | 长操作的进度上报（core 只定义怎么报，不决定报给谁） | ✅ |
//! | [`git`] | **git 命令的唯一出口**（含带凭据执行） | ✅ |
//! | [`hub`] | hub 定位（向上找标记文件）、初始化、配置读写 | ✅ |
//! | [`repo`] | 仓库管理：clone / 列出 / 移除 / 主干分支探测 | ✅ |
//! | [`env`] | 环境：标识 ↔ 分支名解耦映射（设计方案 §2.2） | ✅ |
//! | `workspace` | 需求工作区生命周期：new / get / add / drop / rm / st / push / pull / merge | ✅ |
//! | [`deploy`] | 发布地址矩阵（`deployments.json`） | ✅ |
//! | [`gitlab`] | GitLab API 客户端：projects / branches / MR / users | ✅ |
//! | [`http`] | 极小的 HTTP 封装（JSON 读取与带进度下载，GitLab 与自更新共用） | ✅ |
//! | [`dochub`] | doc-hub 集成：分支、目录契约、`meta.json` 维护 | ✅ |
//! | [`update`] | 自更新（从 GitHub Releases） | ✅ |
//!
//! 完整设计见仓库内 `docs/设计方案.md`。

pub mod date;
pub mod deploy;
pub mod dochub;
pub mod env;
pub mod error;
pub mod git;
pub mod gitlab;
pub mod http;
pub mod hub;
pub mod name;
pub mod paths;
pub mod progress;
pub mod repo;
pub mod schema;
pub mod update;
pub mod workspace;

pub use error::{Error, Result};
pub use hub::Hub;