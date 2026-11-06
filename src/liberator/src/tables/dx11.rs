pub static DX11_Y9S3_79429095: &[(u64, u64)] = &[
    (0x5249E0, 0x40B2D80),
    (0x53193E0, 0x787ECC0),
    (0x53195F0, 0x787EED0),
    (0x531BF00, 0x78817E0),
    (0x5F2EDD7, 0x8FFA537),
    (0xB062FBC, 0xAF25E0C),
    (0xB178588, 0xB287EF0),
    (0xB3F4170, 0xB130170),
    (0xB8A8058, 0xB9580E8),
    (0xBA6E208, 0xBA17B18),
    (0xBA92A88, 0xBA3C3C8),
    (0xBB7C6E8, 0xBB26168),
];

pub static DX11_Y9S4_82873685: &[(u64, u64)] = &[
    (0x1F569F7, 0x12FFA7),
    (0x781DA00, 0x8260E50),
    (0x781DC10, 0x8261060),
    (0x7820090, 0x82634E0),
    (0xBD776E0, 0x42DCB00),
    (0xC9B9F88, 0xC8C6C18),
    (0xCD46530, 0xCC49530),
    (0xCDAB464, 0xCACB274),
    (0xD24DED8, 0xD3AEDC8),
    (0xD482958, 0xD4320D8),
    (0xD4A6668, 0xD455DE8),
    (0xD51D7A0, 0xD4CC5E0),
];

pub static DX11_BUILDS: &[RendererBuild] = &[
    RendererBuild { build: "Y9S3_79429095", rvas: DX11_Y9S3_79429095, patches: &[] },
    RendererBuild { build: "Y9S4_82873685", rvas: DX11_Y9S4_82873685, patches: &[] },
];
