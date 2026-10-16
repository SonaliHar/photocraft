//! UNRUN: actual sixteen nonempty originals remain producer storage references;
//! decoded malformed mutations below are external harnesses, not paid receipts.
use super::*;
use crate::xcf_filters::{NeighborhoodDomain as D, PointReplaceFamily as F};
use crate::xcf_point_replace_source::masked_stock_source as masked;
use photocraft_raster::backing::{BackingQuota, QuotaKind, QuotaLimits, ReadControl};
use photocraft_xcf::ArgumentValue;
use sha2::{Digest, Sha256};
#[derive(Clone, Copy)]
struct Binding {
    path: &'static str,
    bytes: usize,
    sha256: &'static str,
}
struct Case {
    source: Binding,
    capture: Binding,
    packet: Binding,
    kind: masked::Kind,
    precision: u32,
    native_offset: usize,
    native_bytes: usize,
    native_sha: &'static str,
    mask_offset: usize,
    mask_bytes: usize,
    mask_sha: &'static str,
}
const CASES: [Case; 16] = [
    Case {
        source: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-invert-linear-all.xcf",
            bytes: 1959,
            sha256: "2c1a0f6d6493a36d6fcb45bd5f6063f5b3d5ddc2c68b80c93e8f55c2e3b51804",
        },
        capture: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-invert-linear-all.case.json",
            bytes: 46404,
            sha256: "470e74db54eaade9afe58557f939633e7875801806dc209805fa1ba388f4d8b1",
        },
        packet: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-invert-linear-all.captures.bin",
            bytes: 37742,
            sha256: "a5e708c0039d5c74a7ee680cbe87d63c28e3e0226f351870bf88d1add358b26e",
        },
        kind: masked::Kind::InvertLinear,
        precision: 650,
        native_offset: 22490,
        native_bytes: 768,
        native_sha: "517533a7f41f1bfb3d45293ae32fa9dc06590a9f996cdf7d4f27ff9187511490",
        mask_offset: 23258,
        mask_bytes: 192,
        mask_sha: "c90489868a1a64a74239d5331b18d608ef69e28579f36f3ae1949cfe7354907a",
    },
    Case {
        source: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-invert-linear-half-rectangle.xcf",
            bytes: 2003,
            sha256: "3c6ba145dc86281bffd8ad8d5a49e7b2c9e8383b3545683db1ae25f819775af5",
        },
        capture: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-invert-linear-half-rectangle.case.json",
            bytes: 47692,
            sha256: "444c9920430facfefe5a769d6c9094396aaca37ab4cea0cf0c8aedda4d710c85",
        },
        packet: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-invert-linear-half-rectangle.captures.bin",
            bytes: 37742,
            sha256: "8b59f6e1d85653d5ce358207677722f4076239f19e9a7d654b913626ad56c2a8",
        },
        kind: masked::Kind::InvertLinear,
        precision: 650,
        native_offset: 22490,
        native_bytes: 768,
        native_sha: "517533a7f41f1bfb3d45293ae32fa9dc06590a9f996cdf7d4f27ff9187511490",
        mask_offset: 23258,
        mask_bytes: 192,
        mask_sha: "328bc8dcc82f8e7592a7187dd0be3bb368cb9381760cd3df1cdc1126a0b1753b",
    },
    Case {
        source: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-invert-gamma-all.xcf",
            bytes: 1956,
            sha256: "d4346317ca18ee5806b56e8b7c2aa2865d483b0a22f55386d56259cad0ed7443",
        },
        capture: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-invert-gamma-all.case.json",
            bytes: 46279,
            sha256: "cadef0e59169a5fb8c51447ccea45fb8231b5e4bb06e839fef13274cc842b9f6",
        },
        packet: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-invert-gamma-all.captures.bin",
            bytes: 37742,
            sha256: "f3775abfbe5b3280817b0d78d28527aa09c98e9244ea6ebf106bd355fc8e5327",
        },
        kind: masked::Kind::InvertGamma,
        precision: 650,
        native_offset: 22490,
        native_bytes: 768,
        native_sha: "517533a7f41f1bfb3d45293ae32fa9dc06590a9f996cdf7d4f27ff9187511490",
        mask_offset: 23258,
        mask_bytes: 192,
        mask_sha: "c90489868a1a64a74239d5331b18d608ef69e28579f36f3ae1949cfe7354907a",
    },
    Case {
        source: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-invert-gamma-half-rectangle.xcf",
            bytes: 2000,
            sha256: "bc41054be92120832405ecc908712cbfa862a4e72d221132f8ccc2f965832563",
        },
        capture: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-invert-gamma-half-rectangle.case.json",
            bytes: 47566,
            sha256: "ee47f45e94de15ca864d3e74bd8212c77b88dce1a432d7880fc7b9db10008cb8",
        },
        packet: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-invert-gamma-half-rectangle.captures.bin",
            bytes: 37742,
            sha256: "9cc322a485be163a5c201c2a2f60e1ad00b6466a09080b2fff560cfc141a5f29",
        },
        kind: masked::Kind::InvertGamma,
        precision: 650,
        native_offset: 22490,
        native_bytes: 768,
        native_sha: "517533a7f41f1bfb3d45293ae32fa9dc06590a9f996cdf7d4f27ff9187511490",
        mask_offset: 23258,
        mask_bytes: 192,
        mask_sha: "328bc8dcc82f8e7592a7187dd0be3bb368cb9381760cd3df1cdc1126a0b1753b",
    },
    Case {
        source: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-brightness-contrast-all.xcf",
            bytes: 2037,
            sha256: "86976b140835febd2c9294da312203323ce20d51b6af53e886d8b73ccbd7958f",
        },
        capture: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-brightness-contrast-all.case.json",
            bytes: 50503,
            sha256: "eece0816bca7ccf15d15b227dae715a027f86749327b457ca728a3144246dce3",
        },
        packet: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-brightness-contrast-all.captures.bin",
            bytes: 37742,
            sha256: "05aced48f50bfcbec0432d1d24fbd6c55c1cbe24cef4d15f4b0c51a5dda478bc",
        },
        kind: masked::Kind::BrightnessContrast,
        precision: 650,
        native_offset: 22490,
        native_bytes: 768,
        native_sha: "517533a7f41f1bfb3d45293ae32fa9dc06590a9f996cdf7d4f27ff9187511490",
        mask_offset: 23258,
        mask_bytes: 192,
        mask_sha: "c90489868a1a64a74239d5331b18d608ef69e28579f36f3ae1949cfe7354907a",
    },
    Case {
        source: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-brightness-contrast-half-rectangle.xcf",
            bytes: 2081,
            sha256: "2bea3c21fa170a09258679d15b7338d5b1414fb396a9e4531a2a82afb3b21081",
        },
        capture: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-brightness-contrast-half-rectangle.case.json",
            bytes: 51790,
            sha256: "7c23717cd24c4984324d084183e23fe307410b92199facb273c9a4567140b96d",
        },
        packet: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-brightness-contrast-half-rectangle.captures.bin",
            bytes: 37742,
            sha256: "d0a8d059d5f53ed18de4ca8b8f1669703f52fccbb0b072493703cb6ebf92c436",
        },
        kind: masked::Kind::BrightnessContrast,
        precision: 650,
        native_offset: 22490,
        native_bytes: 768,
        native_sha: "517533a7f41f1bfb3d45293ae32fa9dc06590a9f996cdf7d4f27ff9187511490",
        mask_offset: 23258,
        mask_bytes: 192,
        mask_sha: "328bc8dcc82f8e7592a7187dd0be3bb368cb9381760cd3df1cdc1126a0b1753b",
    },
    Case {
        source: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-levels-all.xcf",
            bytes: 2050,
            sha256: "d5ba44f3edca1b932c64c19c29526f7393a16442a2c24f254da646ac5b94139b",
        },
        capture: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-levels-all.case.json",
            bytes: 52561,
            sha256: "6bb8304ee3212a8e1df1ccc264a6ae7fd420aac0797d5835a4c018f3732b2279",
        },
        packet: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-levels-all.captures.bin",
            bytes: 37742,
            sha256: "f00837702b44093075c1f4b3de4b0bf4063320e6efd7ae7808d026135cfdb7f4",
        },
        kind: masked::Kind::Levels,
        precision: 650,
        native_offset: 22490,
        native_bytes: 768,
        native_sha: "517533a7f41f1bfb3d45293ae32fa9dc06590a9f996cdf7d4f27ff9187511490",
        mask_offset: 23258,
        mask_bytes: 192,
        mask_sha: "c90489868a1a64a74239d5331b18d608ef69e28579f36f3ae1949cfe7354907a",
    },
    Case {
        source: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-levels-half-rectangle.xcf",
            bytes: 2094,
            sha256: "9f3617cbea1310c139c1357c4d3b32755195c3e92d41acae31ae0301f1ebfb39",
        },
        capture: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-levels-half-rectangle.case.json",
            bytes: 53848,
            sha256: "f4bce1781f879324f5faaf14a57e4850e23fc6f05706631a244a5e933d6cf871",
        },
        packet: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-levels-half-rectangle.captures.bin",
            bytes: 37742,
            sha256: "c7676f36a3c2802edb16932b718496a3552708423f4a42f6bd3e002bd8a1eff3",
        },
        kind: masked::Kind::Levels,
        precision: 650,
        native_offset: 22490,
        native_bytes: 768,
        native_sha: "517533a7f41f1bfb3d45293ae32fa9dc06590a9f996cdf7d4f27ff9187511490",
        mask_offset: 23258,
        mask_bytes: 192,
        mask_sha: "328bc8dcc82f8e7592a7187dd0be3bb368cb9381760cd3df1cdc1126a0b1753b",
    },
    Case {
        source: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-invert-linear-all.xcf",
            bytes: 1961,
            sha256: "285b09ae4634d8d70de7d125197017f190dc68081eea56536b7a4bff7c89d9e8",
        },
        capture: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-invert-linear-all.case.json",
            bytes: 46462,
            sha256: "077593ea89fca92e4cebdf57bb8d876139fadc4f2c6291da9df4826ad3f0b142",
        },
        packet: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-invert-linear-all.captures.bin",
            bytes: 47534,
            sha256: "0eb429112a3322fc3c5c0e16a5ceca8edf7febb2d13df6ec1413472643f60889",
        },
        kind: masked::Kind::InvertLinear,
        precision: 750,
        native_offset: 27290,
        native_bytes: 1536,
        native_sha: "cfdc6515db50f2d556161b74bce6c773564651fd572934abcf9946d8a0a3ef7c",
        mask_offset: 28826,
        mask_bytes: 384,
        mask_sha: "c759d5450acc8b2acdbca3603451ae30536270e2dab847365650392d561cd68b",
    },
    Case {
        source: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-invert-linear-half-rectangle.xcf",
            bytes: 2093,
            sha256: "b5a393447c7f5da14bebf48e5236503be66b32ed1ab3a05e3f040a48a58afd7d",
        },
        capture: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-invert-linear-half-rectangle.case.json",
            bytes: 47749,
            sha256: "1239e23f68d9dace65031317476eb7e4f42fb57f68d233750903855c1a402d7b",
        },
        packet: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-invert-linear-half-rectangle.captures.bin",
            bytes: 47534,
            sha256: "041db68976f014b11d664d3106647f782e959e23e8e0be897a18f6e479f78ad5",
        },
        kind: masked::Kind::InvertLinear,
        precision: 750,
        native_offset: 27290,
        native_bytes: 1536,
        native_sha: "cfdc6515db50f2d556161b74bce6c773564651fd572934abcf9946d8a0a3ef7c",
        mask_offset: 28826,
        mask_bytes: 384,
        mask_sha: "40bd69272655679d5e880b39181b81f4793da10364d53c9976bad4826acc91d3",
    },
    Case {
        source: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-invert-gamma-all.xcf",
            bytes: 1958,
            sha256: "b81fd18bb1bd07c6de98da8a89e388d71a3cad1fcefd382ada584d2bfa2bf5d1",
        },
        capture: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-invert-gamma-all.case.json",
            bytes: 46337,
            sha256: "38eee7c87af6275a03f0e2ae6a1b9c0143125ec990d89cdf3d5a979bf6ff1cf7",
        },
        packet: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-invert-gamma-all.captures.bin",
            bytes: 47534,
            sha256: "470117ab1a7e596ed6a7464bbb700a529ae076299e552d86b234739c174e4d39",
        },
        kind: masked::Kind::InvertGamma,
        precision: 750,
        native_offset: 27290,
        native_bytes: 1536,
        native_sha: "cfdc6515db50f2d556161b74bce6c773564651fd572934abcf9946d8a0a3ef7c",
        mask_offset: 28826,
        mask_bytes: 384,
        mask_sha: "c759d5450acc8b2acdbca3603451ae30536270e2dab847365650392d561cd68b",
    },
    Case {
        source: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-invert-gamma-half-rectangle.xcf",
            bytes: 2090,
            sha256: "28d9741433bfe1383317f8f7d2b09a99c38981d1240b416fab4931a48b7a0d81",
        },
        capture: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-invert-gamma-half-rectangle.case.json",
            bytes: 47625,
            sha256: "6731c0373e66d527c1106f7ebe15c436c93b6849367c45fc82f24a79e8f8f60c",
        },
        packet: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-invert-gamma-half-rectangle.captures.bin",
            bytes: 47534,
            sha256: "68359ebfc2471ac592e7b70310515edc99b066392df28f3d2f6e373519343031",
        },
        kind: masked::Kind::InvertGamma,
        precision: 750,
        native_offset: 27290,
        native_bytes: 1536,
        native_sha: "cfdc6515db50f2d556161b74bce6c773564651fd572934abcf9946d8a0a3ef7c",
        mask_offset: 28826,
        mask_bytes: 384,
        mask_sha: "40bd69272655679d5e880b39181b81f4793da10364d53c9976bad4826acc91d3",
    },
    Case {
        source: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-brightness-contrast-all.xcf",
            bytes: 2039,
            sha256: "b9dc17861f86823f470305940f35fefc95d7ce50236a8ad1e2147a6b7043f69a",
        },
        capture: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-brightness-contrast-all.case.json",
            bytes: 50562,
            sha256: "d5fb681b7b86c778d717524b1ac467bb8e25de4feda7fd205c2f4292643464a7",
        },
        packet: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-brightness-contrast-all.captures.bin",
            bytes: 47534,
            sha256: "442e218477602ded946c084d38ef5a623bde4f2b0f7a317e372068465bca6e35",
        },
        kind: masked::Kind::BrightnessContrast,
        precision: 750,
        native_offset: 27290,
        native_bytes: 1536,
        native_sha: "cfdc6515db50f2d556161b74bce6c773564651fd572934abcf9946d8a0a3ef7c",
        mask_offset: 28826,
        mask_bytes: 384,
        mask_sha: "c759d5450acc8b2acdbca3603451ae30536270e2dab847365650392d561cd68b",
    },
    Case {
        source: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-brightness-contrast-half-rectangle.xcf",
            bytes: 2171,
            sha256: "7e11f153864d73317a3b60c49870d0e69ec5233028b62827c89415cd93531033",
        },
        capture: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-brightness-contrast-half-rectangle.case.json",
            bytes: 51849,
            sha256: "04952afe69748fdf30885c798108065d8517da0973b5f156250c67cedb0dc0bf",
        },
        packet: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-brightness-contrast-half-rectangle.captures.bin",
            bytes: 47534,
            sha256: "d485d6b7483337b8e15ed8f7fda6ec7c1d07a97045a1cf3763594e95ea32c8ba",
        },
        kind: masked::Kind::BrightnessContrast,
        precision: 750,
        native_offset: 27290,
        native_bytes: 1536,
        native_sha: "cfdc6515db50f2d556161b74bce6c773564651fd572934abcf9946d8a0a3ef7c",
        mask_offset: 28826,
        mask_bytes: 384,
        mask_sha: "40bd69272655679d5e880b39181b81f4793da10364d53c9976bad4826acc91d3",
    },
    Case {
        source: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-levels-all.xcf",
            bytes: 2052,
            sha256: "9ef5710b18672660c2a5f21a5fe109043cdf0894d8c3fec80459f0709d5d7ee9",
        },
        capture: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-levels-all.case.json",
            bytes: 52620,
            sha256: "73156a056699c3e2e7a5fe94e766aa1d2e2ac2f5eff6062d5f803243263f5c26",
        },
        packet: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-levels-all.captures.bin",
            bytes: 47534,
            sha256: "865edc591630f1f53b2bdb728e80d1f84b58a19fb1a47eda51cd615a0315510f",
        },
        kind: masked::Kind::Levels,
        precision: 750,
        native_offset: 27290,
        native_bytes: 1536,
        native_sha: "cfdc6515db50f2d556161b74bce6c773564651fd572934abcf9946d8a0a3ef7c",
        mask_offset: 28826,
        mask_bytes: 384,
        mask_sha: "c759d5450acc8b2acdbca3603451ae30536270e2dab847365650392d561cd68b",
    },
    Case {
        source: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-levels-half-rectangle.xcf",
            bytes: 2184,
            sha256: "af72c4f57d8bdba48ef1a2082a95a24e05c9788b8cf8ff8500f395698873ca6a",
        },
        capture: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-levels-half-rectangle.case.json",
            bytes: 53907,
            sha256: "5036b90c5d22384716d498e4e3b33845d6956c079a55d98ca5fa280ac0c438d1",
        },
        packet: Binding {
            path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-levels-half-rectangle.captures.bin",
            bytes: 47534,
            sha256: "d4817c5c421d409124327f33366ce4e7185ec1f284dfbe38d6418e29c1c470c6",
        },
        kind: masked::Kind::Levels,
        precision: 750,
        native_offset: 27290,
        native_bytes: 1536,
        native_sha: "cfdc6515db50f2d556161b74bce6c773564651fd572934abcf9946d8a0a3ef7c",
        mask_offset: 28826,
        mask_bytes: 384,
        mask_sha: "40bd69272655679d5e880b39181b81f4793da10364d53c9976bad4826acc91d3",
    },
];
const NONE: [Binding; 8] = [
    Binding {
        path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-invert-linear-none.xcf",
        bytes: 1959,
        sha256: "f73e14d719c4b53aeb69fcbc67e756a2f62e25d5eb17828f6ecbf080f98431ae",
    },
    Binding {
        path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-invert-gamma-none.xcf",
        bytes: 1956,
        sha256: "0e777b62d1770e5071a1ba9324adac89926b03e51c6c92d48422aad039103328",
    },
    Binding {
        path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-brightness-contrast-none.xcf",
        bytes: 2037,
        sha256: "f86d61061a1ac97300c16ff2caa95d784ba247bb53f6222eebd087917a5a0f13",
    },
    Binding {
        path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f32-levels-none.xcf",
        bytes: 2050,
        sha256: "b086b48632b7dcb413d5d76df9286247b5d6fd2a60cad946b88d8109acfe2bc7",
    },
    Binding {
        path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-invert-linear-none.xcf",
        bytes: 1961,
        sha256: "ff92e3a4896957dcca7704c2c3ec14503666e28a9f6a0c77c2bf3cdc7ec966f5",
    },
    Binding {
        path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-invert-gamma-none.xcf",
        bytes: 1958,
        sha256: "3c919d76eab58778d02214b2e035e517b11f2a868ba73ff580ac35209abd8b1f",
    },
    Binding {
        path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-brightness-contrast-none.xcf",
        bytes: 2039,
        sha256: "0441e5ac768c5927f9ce0108d0ad2b9a9d36ea65ff4bb69f1e70f853418c7f82",
    },
    Binding {
        path: "temp/sol1-four-masked-points-released-20261016/author-v1/outputs/f64-levels-none.xcf",
        bytes: 2052,
        sha256: "7cc4f84b8671f76c13954b5ca7adce30ed88a3a2a3935a8b510cdf0580d969c4",
    },
];

fn read(b: Binding) -> Vec<u8> {
    let bts = std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(b.path)).unwrap();
    assert_eq!(bts.len(), b.bytes);
    assert_eq!(format!("{:x}", Sha256::digest(&bts)), b.sha256);
    bts
}
fn decoded(i: usize) -> XcfFile {
    photocraft_xcf::decode(&read(CASES[i].source), &Default::default()).unwrap()
}
fn budget() -> photocraft_xcf::Budget<'static> {
    photocraft_xcf::Budget::new(photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(), None)
}
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn selected(f: &XcfFile) -> SelectedStagedFamily {
    SourceSelectedPolicy::family_controlled(f, &Interrupt::NONE, Some(&budget())).unwrap()
}
fn mask_words(f: &mut XcfFile, bits: u64) {
    let sample = f.sample;
    let m = f.layers[0].effects[0].mask.as_mut().unwrap().plane.as_mut().unwrap();
    let word = if sample == Sample::F64 { bits.to_ne_bytes().to_vec() } else { (bits as u32).to_ne_bytes().to_vec() };
    m.bytes = word.repeat(m.width as usize * m.height as usize);
}
#[test]
#[ignore = "UNRUN: sixteen genuine all/half originals actualreader->Point/Source/native/mask/raw/property custody, exact0/0/2/4F32 order and samequota refund; no rendered numerical theorem"]
fn genuine_sixteen_sources_keep_raw_and_native() {
    use photocraft_doc::{
        LayerContent,
        item_identity::ItemRef,
        xcf_filters::{ExactNumber, FilterValue},
    };
    for case in &CASES {
        let bytes = read(case.source);
        let _capture = read(case.capture);
        let packet = read(case.packet);
        let raw = photocraft_xcf::decode(&bytes, &Default::default()).unwrap();
        assert_eq!(selected(&raw), SelectedStagedFamily::Point(case.kind.family()));
        assert!(masked::qualify(&raw, &Interrupt::NONE, Some(&budget())).unwrap());
        let native_raw = &raw.layers[0].plane.as_ref().unwrap().bytes;
        let mask_raw = &raw.layers[0].effects[0].mask.as_ref().unwrap().plane.as_ref().unwrap().bytes;
        assert_eq!(native_raw, packet.get(case.native_offset..case.native_offset + case.native_bytes).unwrap());
        assert_eq!(format!("{:x}", Sha256::digest(native_raw)), case.native_sha);
        assert_eq!(mask_raw, packet.get(case.mask_offset..case.mask_offset + case.mask_bytes).unwrap());
        assert_eq!(format!("{:x}", Sha256::digest(mask_raw)), case.mask_sha);
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 64 << 20, scratch: 0 });
        let c = ctl();
        let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap();
        let source = crate::import_owned_xcf_from_reader_in_quota(
            "genuine masked scalar",
            &mut std::io::Cursor::new(&bytes),
            bytes.len() as u64,
            &Default::default(),
            &limits,
            q.clone(),
            &c,
        )
        .unwrap();
        let crate::OwnedXcfImport::Point(source) = source else { panic!("actual Point") };
        assert!(source.document().is_readonly_point_storage());
        let doc = source.document().document();
        let state = doc.xcf_state.as_ref().unwrap();
        assert_eq!((doc.layers.len(), state.filter_stacks.len(), state.filter_masks.len(), state.items.len()), (1, 1, 1, 2));
        assert_eq!(doc.source_color_context.as_ref().unwrap().source_precision, case.precision);
        assert!(doc.source_color_context.as_ref().unwrap().original_icc.is_none());
        let layer = &doc.layers[0];
        let node = &state.filter_stacks[0].nodes[0];
        let effect = &raw.layers[0].effects[0];
        assert_eq!(node.operation.name, effect.operation);
        assert_eq!(node.operation.stored_version, "0:0");
        assert_eq!(node.operation.schema_epoch, case.kind.epoch());
        assert_eq!(node.composition.as_ref().unwrap().compatibility_epoch, case.kind.epoch());
        assert_eq!(node.opacity_f64_bits, 1f64.to_bits());
        assert!(node.enabled);
        assert_eq!(node.arguments.len(), effect.arguments.len());
        for (a, b) in node.arguments.iter().zip(&effect.arguments) {
            assert_eq!(a.name, b.name);
            let ArgumentValue::Float32Bits(bits) = b.value else { panic!("stored F32") };
            assert_eq!(a.value, FilterValue::Number(ExactNumber::Float32Bits(bits)));
        }
        for p in &effect.properties {
            let identity = format!("{}@{}:{}", p.tag, p.offset, p.declared_bytes);
            let kept = state
                .opaque
                .iter()
                .find(|r| r.owner == Some(ItemRef::Layer(layer.id)) && r.namespace == "gimp/xcf/source-property" && r.identity == identity)
                .unwrap();
            assert_eq!(kept.bytes.as_slice(), p.payload.as_slice());
        }
        let mask = &state.filter_masks[0];
        assert_eq!(node.mask, Some(ItemRef::Mask(mask.id)));
        assert_eq!(mask.owner, node.id);
        assert_eq!(&mask.surface.try_read_interleaved(mask.bounds).unwrap(), mask_raw);
        let proof = mask.validate_checked_f32_coverage_with(&c).unwrap();
        assert!(proof.raw_nonempty());
        doc.validate_xcf_state_with_checked_filter_mask(&proof, &Interrupt::NONE).unwrap();
        let LayerContent::BackedRaster(native) = &layer.content else { panic!("native") };
        let mut actual = Vec::new();
        let mut pixel = vec![0; if case.precision == 750 { 32 } else { 16 }];
        for y in 0..raw.height {
            for x in 0..raw.width {
                native.checked_view().read_pixel_native_into(x as i32, y as i32, &mut pixel, &c).unwrap();
                actual.extend_from_slice(&pixel);
            }
        }
        assert_eq!(&actual, native_raw);
        if case.precision == 750 {
            assert!(mask_raw.chunks_exact(8).any(|w| u64::from_ne_bytes(w.try_into().unwrap()) == 0x3ff0000009b25010));
        }
        assert!(q.used(QuotaKind::Resident) > 0);
        drop(source);
        assert_eq!(q.used(QuotaKind::Resident), 0);
        assert_eq!(q.used(QuotaKind::Index), 0);
    }
}
#[test]
#[ignore = "UNRUN: eight genuine none controls retain old selection/final validator and are never repaired as masked; signedzero-only and PointStack identity stay old"]
fn empty_and_old_priorities_do_not_upgrade() {
    for source in NONE {
        let raw = photocraft_xcf::decode(&read(source), &Default::default()).unwrap();
        assert!(!masked::hint(&raw, &Interrupt::NONE, &budget()).unwrap());
        assert!(!masked::qualify(&raw, &Interrupt::NONE, Some(&budget())).unwrap());
        let old = SourceSelectedPolicy::family(&raw);
        assert_eq!(selected(&raw), old);
        assert!(matches!(old, SelectedStagedFamily::Point(F::InvertLinear | F::InvertGamma | F::Brightness | F::Levels)));
    }
    let mut f = decoded(0);
    mask_words(&mut f, 0x80000000);
    assert!(!masked::hint(&f, &Interrupt::NONE, &budget()).unwrap());
    assert_eq!(selected(&f), SourceSelectedPolicy::family(&f));
    let mut f = decoded(0);
    f.layers[0].blend_space = 0;
    f.layers[0].composite_space = 2;
    f.layers[0].composite_mode = 1;
    assert_eq!(selected(&f), SourceSelectedPolicy::family(&f));
    assert_ne!(selected(&f), SelectedStagedFamily::Point(F::MaskedInvertLinear));
}
#[test]
#[ignore = "UNRUN: selected malformed params/outer/source failures stay final, rawpositive beforeFloat and full preimage domain preserve bits, noSourceHDR/Gray/opacity/topology grant"]
fn exact_domains_and_selected_failure_are_closed() {
    for i in 0..CASES.len() {
        let original = decoded(i);
        let e = &original.layers[0].effects[0];
        assert!(masked::parameters(e) == Some(CASES[i].kind));
        for bad in 0..9 {
            let mut f = original.clone();
            match bad {
                0 => f.layers[0].effects[0].version = Some("1:0".into()),
                1 => f.layers[0].effects[0].state.mode = Some(28),
                2 => f.layers[0].effects[0].state.enabled = Some(false),
                3 => f.layers[0].visible = false,
                4 => f.layers[0].opacity = 0.375,
                5 => f.layers[0].effects[0].state.blend_space = None,
                6 => f.icc = vec![1],
                7 => f.layers[0].offset = (1, 0),
                _ => f.layers[0].plane.as_mut().unwrap().bytes.pop().map(|_| ()).unwrap(),
            }
            let old = SourceSelectedPolicy::family(&f);
            if matches!(old, SelectedStagedFamily::Point(F::InvertLinear | F::InvertGamma | F::Brightness | F::Levels)) {
                assert_eq!(selected(&f), SelectedStagedFamily::Point(CASES[i].kind.family()));
            } else {
                assert_eq!(selected(&f), old);
            }

            assert!(!masked::qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
        }
        let mut e = e.clone();
        if e.arguments.is_empty() {
            let mut extra = decoded(4).layers[0].effects[0].arguments[0].clone();
            extra.name = "brightness".into();
            e.arguments.push(extra);
        } else {
            e.arguments[0].value = ArgumentValue::Float64Bits(0);
        };
        assert!(masked::parameters(&e).is_none());
    }
    let mut f = decoded(8);
    assert_eq!(f.sample, Sample::F64);
    mask_words(&mut f, 1);
    assert!(masked::hint(&f, &Interrupt::NONE, &budget()).unwrap());
    assert!(masked::qualify(&f, &Interrupt::NONE, Some(&budget())).unwrap());
    for word in [(-1f64).to_bits(), f64::NAN.to_bits(), f64::INFINITY.to_bits(), 1.01f64.to_bits()] {
        let mut bad = f.clone();
        mask_words(&mut bad, word);
        assert!(!masked::qualify(&bad, &Interrupt::NONE, Some(&budget())).unwrap());
    }
}
#[test]
#[ignore = "UNRUN: original cumulative Source Work/cancel, ordinaryPoint actual2N argument+numeric tuple Resident envelope/capacity, badnative/mask and real quota denial remain effective"]
fn original_ledger_and_new_mapper_fee_are_effective() {
    for i in [0, 4, 6, 8, 12, 14] {
        let f = decoded(i);
        let b = budget();
        masked::qualify(&f, &Interrupt::NONE, Some(&b)).unwrap();
        let used = b.used(photocraft_xcf::BudgetKind::Work);
        masked::qualify(&f, &Interrupt::NONE, Some(&b)).unwrap();
        assert_eq!(b.used(photocraft_xcf::BudgetKind::Work), 2 * used);
        assert!(used >= 512);
        let low = photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap().lowered(&[("sampleWorkUnits", used - 1)]).unwrap();
        assert!(masked::qualify(&f, &Interrupt::NONE, Some(&photocraft_xcf::Budget::new(low, None))).is_err());
        assert!(matches!(masked::qualify(&f, &Interrupt::cancel_only(&|| true), Some(&budget())), Err(IoError::Cancelled)));
        let e = &f.layers[0].effects[0];
        let n = 2 * e.arguments.len();
        let floor = n * std::mem::size_of::<photocraft_doc::xcf_filters::FilterArgument>()
            + n * std::mem::size_of::<(&str, f64)>()
            + std::mem::size_of::<Vec<photocraft_doc::xcf_filters::FilterArgument>>()
            + std::mem::size_of::<Vec<(&str, f64)>>();
        assert!(masked::mapping_bytes(e).unwrap() >= floor);
        let mapped = crate::xcf_filters::qualified_stack(
            std::slice::from_ref(e),
            &[None],
            photocraft_doc::item_identity::ItemRef::Layer(photocraft_doc::LayerId(1)),
            1,
            22,
            D::PointReplace(CASES[i].kind.family()),
        )
        .unwrap();
        assert_eq!(mapped.nodes[0].arguments.len(), e.arguments.len());
    }
    let bytes = read(CASES[0].source);
    let c = ctl();
    let q = BackingQuota::new(QuotaLimits { resident: 1, index: 1, scratch: 0 });
    assert!(
        crate::import_owned_xcf_from_reader_in_quota(
            "quota denied",
            &mut std::io::Cursor::new(&bytes),
            bytes.len() as u64,
            &Default::default(),
            &photocraft_xcf::ResourceLimits::for_policy(Default::default()).unwrap(),
            q.clone(),
            &c
        )
        .is_err()
    );
    assert_eq!(q.used(QuotaKind::Resident), 0);
    assert_eq!(q.used(QuotaKind::Index), 0);
}
