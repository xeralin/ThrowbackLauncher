pub static DX11_Y9S3_79429095: &[(u64, u64)] = &[
    (0xB178588, 0xB287EF0),
    (0xB8A8058, 0xB9580E8),
    (0xBA6E208, 0xBA17B18),
    (0xBA92A88, 0xBA3C3C8),
    (0xBB7C6E8, 0xBB26168),
];

pub static DX11_Y9S4_82873685: &[(u64, u64)] = &[
    (0xC9B9F88, 0xC8C6C18),
    (0xD24DED8, 0xD3AEDC8),
    (0xD482958, 0xD4320D8),
    (0xD4A6668, 0xD455DE8),
    (0xD51D7A0, 0xD4CC5E0),
];

pub static DX11_BUILDS: &[RendererBuild] = &[
    RendererBuild { build: "Y9S3_79429095", rvas: DX11_Y9S3_79429095, patches: &[] },
    RendererBuild { build: "Y9S4_82873685", rvas: DX11_Y9S4_82873685, patches: &[] },
];
