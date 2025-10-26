#![allow(unused)]

use std::sync::Arc;

use async_trait::async_trait;
use rs9p::{
    Data, FCall, GetAttrMask, QId, QIdType, Result, Stat, Time,
    srv::{FId, Filesystem, srv_async},
};
use stringlit::s;

use crate::system::System;

#[derive(Clone)]
pub struct FsOverlay {
    pub sys: Arc<System>,
}

impl FsOverlay {
    pub fn new(sys: System) -> Self {
        Self { sys: Arc::new(sys) }
    }
}

#[derive(Debug, Default)]
pub struct MyFId {
    pub id: String,
}

#[async_trait]
impl Filesystem for FsOverlay {
    type FId = MyFId;

    async fn rattach(
        &self,
        _id: &FId<Self::FId>,
        _afid: Option<&FId<Self::FId>>,
        _uname: &str,
        _aname: &str,
        _n_uname: u32,
    ) -> Result<FCall> {
        Ok(FCall::RAttach {
            qid: QId {
                typ: QIdType::DIR,
                version: 1,
                path: 1,
            },
        })
    }

    async fn rgetattr(&self, fid: &FId<Self::FId>, req_mask: GetAttrMask) -> Result<FCall> {
        Ok(FCall::RGetAttr {
            valid: GetAttrMask::all(),
            qid: QId {
                typ: QIdType::DIR,
                version: 1,
                path: 1,
            },
            stat: Stat {
                mode: 0777,
                uid: 1000,
                gid: 1000,
                nlink: 0,
                rdev: 0,
                size: 0,
                blksize: 0,
                blocks: 0,
                atime: Time { sec: 0, nsec: 0 },
                mtime: Time { sec: 0, nsec: 0 },
                ctime: Time { sec: 0, nsec: 0 },
            },
        })
    }

    async fn rclunk(&self, _id: &FId<Self::FId>) -> Result<FCall> {
        Ok(FCall::RClunk)
    }

    async fn rwalk(
        &self,
        _: &FId<Self::FId>,
        _new: &FId<Self::FId>,
        _wnames: &[String],
    ) -> Result<FCall> {
        Ok(FCall::RWalk { wqids: vec![] })
    }

    async fn rread(&self, _: &FId<Self::FId>, _offset: u64, _count: u32) -> Result<FCall> {
        Ok(FCall::RRead {
            data: Data(vec![1, 2, 3, 4]),
        })
    }
}
