// ============================================================
// Single source of truth for Docker registry mirrors
// ============================================================

/// One known public Docker registry mirror.
pub struct MirrorInfo {
    pub name: &'static str,
    pub url: &'static str,
    pub desc: &'static str,
}

/// Known public Docker registry mirrors (single definition;
/// used by both the CLI mirror auto-detect and the GUI settings tab).
pub fn known_mirrors() -> &'static [MirrorInfo] {
    &[
        MirrorInfo {
            name: "USTC",
            url: "https://docker.mirrors.ustc.edu.cn",
            desc: "China - University of Science and Technology",
        },
        MirrorInfo {
            name: "Tencent Cloud",
            url: "https://mirror.ccs.tencentyun.com",
            desc: "China - Tencent Cloud",
        },
        MirrorInfo {
            name: "DaoCloud",
            url: "https://2a59f68c.m.daocloud.io",
            desc: "China - DaoCloud",
        },
        MirrorInfo {
            name: "Docker CN",
            url: "https://registry.docker-cn.com",
            desc: "China - Docker Official CN Mirror",
        },
        MirrorInfo {
            name: "DockerHub Proxy",
            url: "https://dockerhub.timeweb.cloud",
            desc: "Russia - Timeweb Cloud",
        },
    ]
}

/// Mirror URLs only, in probe order (used by mirror auto-detection).
pub fn registry_mirror_urls() -> impl Iterator<Item = &'static str> {
    known_mirrors().iter().map(|m| m.url)
}
