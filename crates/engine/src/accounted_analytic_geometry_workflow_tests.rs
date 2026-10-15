//! UNRUN authentic 256-case four-operator assigned-ICC geometry workflows.
//! Source and archive Vecs are external captures. Production import/native/model
//! owners retain actual quota receipts; no rendering-equality oracle is invented.
use super::*;
use photocraft_doc::LayerContent;
use photocraft_raster::{
    Interrupt,
    backing::{
        NativePlaneDescriptor,
        quota::{BackingQuota, QuotaKind, QuotaLimits},
    },
    checked_view::NativeTileBytes,
};
use sha2::{Digest, Sha256};
use std::{io::Cursor, path::PathBuf, sync::Arc};
fn ctl() -> ReadControl<'static> {
    ReadControl { interrupt: &Interrupt::NONE, work: &|_| Ok(()) }
}
fn hash(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|v| format!("{v:02x}")).collect()
}
const CASES: [(&str, &str, usize, i32); 256] = [
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f32-op1.xcf", "fd97c954dbfee8fa69695a69ce1e400499f2854a4f5af2acf75bb4a99d708b9b", 9339, 1),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f32-op1.edit.xcf", "e6ee1e8f20dcc203230ebe3ca6e97e249bac7180e3fea6c0ffd3aec3c6d1e784", 9339, 1),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f32-op2.xcf", "ab0f494fef20f85d0d1e93b81790dba410c914be2d75e5860667b973ae75ef1e", 9339, 2),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f32-op2.edit.xcf", "d25cc34f5c1f7675c78b95c18aacf3bb56de8f96fbf7bd8da7f66eec60fe8471", 9339, 2),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f32-op3.xcf", "6280cb35f6495f2d6113cfaf6a1befb28042ad9734d03b3a38727261ee62559d", 9339, 3),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f32-op3.edit.xcf", "386af615d1dcd576f04d76450278cf868c161fd1c1ee5c55ad36719e7291ca12", 9339, 3),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f32-op4.xcf", "43bcfecb913670e4aa3d738daa71cc3a69f1b39ca80c61a1bbb9adcf0802a81d", 9339, 4),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f32-op4.edit.xcf", "d693eadda4df69d8768953dec889fadf1dd8e219f9e4c99d0152e7242a7217ae", 9339, 4),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f64-op1.xcf", "d09764e320785b11fe20ad38ae29a0a228b8422891e9dd2e6c114762297a9788", 13710, 1),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f64-op1.edit.xcf", "41ebbd873c690294822ecc5465ffad0e5e5913e0f2bcfb04801a324acad4290d", 13706, 1),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f64-op2.xcf", "a40f9837d56038faed84e09cf5b5127f31843154ecb15ca8762901932386ca02", 13710, 2),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f64-op2.edit.xcf", "d82d9a4e683f9be2dbecb2a5f41bd32c0656be17ed2c673194cc61db5ff69515", 13706, 2),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f64-op3.xcf", "d8358732abedfbc7e9a5261349a60f3446aac514d3cab4533f02e75f6c5f5b31", 13710, 3),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f64-op3.edit.xcf", "917dee9ec0d6716b3ecbd2a3f88ec47540395091d8a8b6f5f44a8aa679d2ac67", 13706, 3),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f64-op4.xcf", "605ebdf92089ef77927d3a39581db3d84b741ad9df57fd7b7a930f350c93f6cb", 13710, 4),
    ("normal-mask-geometry-holdout-evidence/left-top/rgb-f64-op4.edit.xcf", "05f53b4e41eca554794b9f70bada1972e82ff1a38cb5e5067b775431b4037345", 13706, 4),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f32-op1.xcf", "b356d0639fbc7dff8a81fd8a8444356586088233cc8f423f7cc516a205af2461", 5247, 1),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f32-op1.edit.xcf", "87c3f95310e5b444c4b4fb07180483c82a62090e33af4e203c301db33955e8b5", 5247, 1),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f32-op2.xcf", "c88f868af205e468030cd8ecc4d479f0ecbf36a50cafbe3a43d80b5d01b13642", 5247, 2),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f32-op2.edit.xcf", "b901130f3c62273b772248027ee044d5fc2fc5134811e1d079e0746e9704df9a", 5247, 2),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f32-op3.xcf", "c54efc051531d8372f56444cb6408eacda687232f1e8519899e21f0a5e590d3c", 5247, 3),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f32-op3.edit.xcf", "bf4966d57311bf6117c4768d74bb7e393998a47d1f597e2ee2e43f00b9bd29ce", 5247, 3),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f32-op4.xcf", "64163f76c08404904803fed411f44f424efe7f91c6888529a28413a115695e31", 5247, 4),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f32-op4.edit.xcf", "4cb176b62c7b865709f900e66c7dacb42d77515bd91a642e4bd6b3feb3d253e7", 5247, 4),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f64-op1.xcf", "e4f455f58216921cffc088d0116b21040fae4bc232a20e073d0ac94aa5c5d81c", 6690, 1),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f64-op1.edit.xcf", "309e2249f263256046ee997d625e9bab3077271dfdf2a90bdfb7f5c31572da9d", 6686, 1),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f64-op2.xcf", "dcf0b6ddbe2c3d9a899a844224a85b3a5411864cd1ec730ae2bac069a624276f", 6690, 2),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f64-op2.edit.xcf", "297d587f1de6ea05d356b804244a58097bf165c159c61298ce2a584760e3b271", 6686, 2),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f64-op3.xcf", "c85f4a32e60d4fa50eb012e3a1f625dce6c9ed5f452c132e06a21e7b8ede29ca", 6690, 3),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f64-op3.edit.xcf", "a6d6c24059023795168a064c3c697d6114d7481ceac773606a6ce2bc0c060999", 6686, 3),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f64-op4.xcf", "cb9dad1b89275d258d1789bfe3ebb45a817a647bd0df50b01be32517ec272a1f", 6690, 4),
    ("normal-mask-geometry-holdout-evidence/left-top/gray-f64-op4.edit.xcf", "b391ca742c3847d9e5d498b8dcceeb68f22a6bd63b5e74ec606e2e1c9a29b04c", 6686, 4),
    ("normal-mask-geometry-holdout-evidence/right-bottom/rgb-f32-op1.xcf", "cd39ff9707616cf497ca9d91a408dd003362d5543313cc3af7a01d755b147b47", 12177, 1),
    ("normal-mask-geometry-holdout-evidence/right-bottom/rgb-f32-op1.edit.xcf", "c7ef420ade131f885b34d4af70a1f666b6b38319a1e56a250027211bbe0bf653", 12177, 1),
    ("normal-mask-geometry-holdout-evidence/right-bottom/rgb-f32-op2.xcf", "aa90a985a90492886867411b37d68c8edc40443610aee28196ff65db2994e2f0", 12177, 2),
    ("normal-mask-geometry-holdout-evidence/right-bottom/rgb-f32-op2.edit.xcf", "6ec773ee30e022218a80210a0d459f5b19d418d54d2fc191d12dd741961e9292", 12177, 2),
    ("normal-mask-geometry-holdout-evidence/right-bottom/rgb-f32-op3.xcf", "7a230e78a838af94580c4b6e2da3a7222dd6581ffba297a82edf40af45a937f5", 12177, 3),
    ("normal-mask-geometry-holdout-evidence/right-bottom/rgb-f32-op3.edit.xcf", "f0e68181a2c4b719f05a785f884b1b3c1e7db2815f101cb3620e3393a42f783b", 12177, 3),
    ("normal-mask-geometry-holdout-evidence/right-bottom/rgb-f32-op4.xcf", "24bd8bd2341bb398feefabf25117d4b324a0d19852d2b9f8325fbb25c657f786", 12177, 4),
    ("normal-mask-geometry-holdout-evidence/right-bottom/rgb-f32-op4.edit.xcf", "d73bb1098a520af076bbc02572a6b294479d3cb6225ad1157e2cc91c8466276e", 12177, 4),
    ("normal-mask-geometry-holdout-evidence/right-bottom/rgb-f64-op1.xcf", "543abefd7dc93bc5f1e9f0278883cfea2b99cbc3fa6f0021e5ee9f859946fa77", 18326, 1),
    ("normal-mask-geometry-holdout-evidence/right-bottom/rgb-f64-op1.edit.xcf", "2acd4181a11caaca0b9fb2e4d86f9cc471e73c7418e734b21fcc97de4e6f700a", 18327, 1),
    ("normal-mask-geometry-holdout-evidence/right-bottom/rgb-f64-op2.xcf", "7e360b47276c8db1951c38d626af34a63b80670f11f8cc605cb8d5d6c625c6f6", 18326, 2),
    ("normal-mask-geometry-holdout-evidence/right-bottom/rgb-f64-op2.edit.xcf", "13a8681c66ca7c7438c54a9698e399af3b643f94fc4e7dd626a6466d871e3dff", 18327, 2),
    ("normal-mask-geometry-holdout-evidence/right-bottom/rgb-f64-op3.xcf", "01aba7ae0e4e3ef4877918501b1561021608d840547e0c55bd0b4980dd2fbf0a", 18326, 3),
    ("normal-mask-geometry-holdout-evidence/right-bottom/rgb-f64-op3.edit.xcf", "96c015f6ea10d0e243b69225a73e0906072f350c7418219a8143fa24ace2aefc", 18327, 3),
    ("normal-mask-geometry-holdout-evidence/right-bottom/rgb-f64-op4.xcf", "f5c76e3ba801b55e95e97709c64e9899bff042dbe2bf3ee2bab37cef50568f0c", 18326, 4),
    ("normal-mask-geometry-holdout-evidence/right-bottom/rgb-f64-op4.edit.xcf", "0cc7883ef8a92d3ab59d932aae246482a7676ed8cb8d4b738c2a73942e5ccaa8", 18327, 4),
    ("normal-mask-geometry-holdout-evidence/right-bottom/gray-f32-op1.xcf", "03110e9b7421b3797785da01d4db64d61bedd9eaf5517fd1fb8059ceefac420c", 6469, 1),
    ("normal-mask-geometry-holdout-evidence/right-bottom/gray-f32-op1.edit.xcf", "e22ca440d4336c93eae88f88b5b7fd6a385f2c763e38ad885f2d73990b04891f", 6469, 1),
    ("normal-mask-geometry-holdout-evidence/right-bottom/gray-f32-op2.xcf", "a9ba93e33b60321f0f5696fb3f2e7746d761cf61708f5a16e350eb55b603ea65", 6469, 2),
    ("normal-mask-geometry-holdout-evidence/right-bottom/gray-f32-op2.edit.xcf", "cd0eef13ae2e1804833aa2135853c71971dcc59751c3e6accf10680abd891457", 6469, 2),
    ("normal-mask-geometry-holdout-evidence/right-bottom/gray-f32-op3.xcf", "80e8f9300e6511a6d74b911aa141aef15e5b0a5d7cc2044405e0fdd86a8f379b", 6469, 3),
    ("normal-mask-geometry-holdout-evidence/right-bottom/gray-f32-op3.edit.xcf", "290c8113484135cfca6cd6b3acba39fd04f44e954edbb2718d58c07e2d4bda5c", 6469, 3),
    ("normal-mask-geometry-holdout-evidence/right-bottom/gray-f32-op4.xcf", "3634799d244d5e51ef1392411fb7f809af4ac25469ab7cf522305d3544608820", 6469, 4),
    ("normal-mask-geometry-holdout-evidence/right-bottom/gray-f32-op4.edit.xcf", "f18e5a0ee072a956cc762b18dcb37163f32a78b75323c4e1479a470646cc9e7d", 6469, 4),
    ("normal-mask-geometry-holdout-evidence/right-bottom/gray-f64-op1.xcf", "26150114e9cab77f81dde5b76e515a2a4a9f5436dfae123236fdd6560403f9d0", 8478, 1),
    ("normal-mask-geometry-holdout-evidence/right-bottom/gray-f64-op1.edit.xcf", "9a9bb62d8451fbb7ea881b23ccbc455d8ced987d94cca068158a9a4ec65b2394", 8479, 1),
    ("normal-mask-geometry-holdout-evidence/right-bottom/gray-f64-op2.xcf", "75b3be1b618a3e7fe2d799072c250c30b4f7b68ea226d953283dca3e3ec8d244", 8478, 2),
    ("normal-mask-geometry-holdout-evidence/right-bottom/gray-f64-op2.edit.xcf", "92f5f900d2cfbad30be62a7c52e3517f08c657e4b5238e89efb5bd86a8a82925", 8479, 2),
    ("normal-mask-geometry-holdout-evidence/right-bottom/gray-f64-op3.xcf", "944d8e1957cca029280cafa4935a73b9bd4ef9aa352ce98a929c514cbecf8906", 8478, 3),
    ("normal-mask-geometry-holdout-evidence/right-bottom/gray-f64-op3.edit.xcf", "235a011502ce2415ea83960788ab5e89ee194783c1871a6fa1e0cb40417d411b", 8479, 3),
    ("normal-mask-geometry-holdout-evidence/right-bottom/gray-f64-op4.xcf", "742a423518f8d3d0f0cbfad52e4e32eed7fdca46d6cf6b28e5b1e5a4b3625169", 8478, 4),
    ("normal-mask-geometry-holdout-evidence/right-bottom/gray-f64-op4.edit.xcf", "47fcfc7ffb2ab2f7a1c903d78557e7a5d940493f50e6732888bf154103433464", 8479, 4),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u16-op1.xcf", "4250f944696df427e92e3e4568aede8f7ec49ff8d9fbf26c429cc2210282d5bf", 6409, 1),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u16-op1.edit.xcf", "e02d72491deadc5cf064eedf300403ec60dbdda4afccc44ac0463c865a1d2a75", 6409, 1),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u16-op2.xcf", "298f7b56c94b3a9acdd4634f8eacc1643b5c361403947b11adb80fa6fea541dd", 6409, 2),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u16-op2.edit.xcf", "5f5aac4253a082d8eefc0976a15722b250f11d4a6cf339d06005612e22fc69cc", 6409, 2),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u16-op3.xcf", "6f99f6247e017b7fd260d6b815695476e44657e30f9294798b2f54f54222d045", 6409, 3),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u16-op3.edit.xcf", "855b9f0963459d690cfa00ff6b682b5ceb4a7c7de231c5ad731c46ec0c2c6521", 6409, 3),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u16-op4.xcf", "6cc984c03caefe3697ca0765787259b2943fbc10cc6a300620bbfd8cdd8f3787", 6409, 4),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u16-op4.edit.xcf", "b114ef42f26f690af13034b1b210e2a2f2a7841ef725545a68c7de4b042c6d90", 6409, 4),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u32-op1.xcf", "b7f941d9e7956fd4cbde707da299d92b9c5ad76131613e9e8589ead1a10894b7", 10477, 1),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u32-op1.edit.xcf", "515d8268a6c2b6edf9b1d83bbce0f9c83ab0d104e9263d44448f40210f00bf4e", 10477, 1),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u32-op2.xcf", "7852a3a531c05ba6085f713fee3d2bde70344563dc67ce0962857c4ee95e2d1a", 10477, 2),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u32-op2.edit.xcf", "8ad4962404aba2376bdfb404b7a96bc4f5e6f2513164f14574d726644abaea44", 10477, 2),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u32-op3.xcf", "32a893d94ff713708cb9ca0182e9d60f018b9b00e4963acc7f8fc9bd8ed95847", 10477, 3),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u32-op3.edit.xcf", "d01e95abdf49245d117a2d905c5dc9ad562a95f8489b3100437cf808ff83ccce", 10477, 3),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u32-op4.xcf", "b41aa7e93a35ad401ee23f58237e181db262adbf7af1e9e4bedeee73bb7f226e", 10477, 4),
    ("normal-mask-geometry-integer-evidence/left-top/rgb-u32-op4.edit.xcf", "c35ecd6be1a83339dafddddc2ff52c90e41003b7ede983a08600fa076fe16aa1", 10477, 4),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u16-op1.xcf", "b39d43a1d3a4de272d655b526c104d90f67bbce6764f5865fe78bec5ab4e31e6", 4261, 1),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u16-op1.edit.xcf", "6cf19963129f4ecdcfd89b432f5a8c8adb281a484a163cd236c4d9d08c472d53", 4261, 1),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u16-op2.xcf", "f906a7defebfce50480c523c2d544535f73b3a2310f12eb26c33364280aeaed3", 4261, 2),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u16-op2.edit.xcf", "993faa17a774c59741b0c453f128e68abf53c567ec0f2754d6b18b88af88ab72", 4261, 2),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u16-op3.xcf", "87b1fb3edcf289fc3452d838a1ff3bb79709e6728babb3de9d48248343e1fdaf", 4261, 3),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u16-op3.edit.xcf", "f309c38399c43a578ee193a0a2f30afcb24686433f024db8e4e35cdc704259c8", 4261, 3),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u16-op4.xcf", "2715b93c6b22d8314c47978fa98bfb346efec3a7198d39629dd9b713423a3422", 4261, 4),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u16-op4.edit.xcf", "7a4fae58c246c707d7a16d2f6fbd054cb3adf9b05066ccbd65caa1d0b5879b2c", 4261, 4),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u32-op1.xcf", "0853c2ff39fc77dcb7aa88a2ab5411789fa08bc202141a9df6c8243e39ed6c4c", 6385, 1),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u32-op1.edit.xcf", "493e5c191750c4899698ce6ce5561aa8c51e5fab8f6182ceb54dd4b2411564a7", 6385, 1),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u32-op2.xcf", "551fcac24063d1ce685404ce00ddade0dad2814c7d511d551a59b76235936f9b", 6385, 2),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u32-op2.edit.xcf", "d4f17ff27a39144d51583f97bcf21bcd5a81c9996c5e5f18d6efa8b272845e24", 6385, 2),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u32-op3.xcf", "2598fe1aaad8a130c40040254d44708128ec17be0cd1ea101cb0bd578029c102", 6385, 3),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u32-op3.edit.xcf", "4f08335af5737055b2ffee34e87c8c4c996492900e3bc4d1028ae9bf9d3b819c", 6385, 3),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u32-op4.xcf", "4e3f10f96df65ec064d0e2139838387312b0b328b62ba35617ffb9d48abbc047", 6385, 4),
    ("normal-mask-geometry-integer-evidence/left-top/gray-u32-op4.edit.xcf", "bcb2e5699a5194ca84a645bff757272b8d3f74977bc47459a9716e059b28b942", 6385, 4),
    ("normal-mask-geometry-integer-evidence/right-bottom/rgb-u16-op1.xcf", "1a3795ffbfd912dc9141dd5473bc55f46eaae1f3737e8f1a6089bd31b56849b3", 8032, 1),
    ("normal-mask-geometry-integer-evidence/right-bottom/rgb-u16-op1.edit.xcf", "8edb2d6495ae52f0d79f538490431951c0ad32879da4f088ffc27303939f3f73", 8032, 1),
    ("normal-mask-geometry-integer-evidence/right-bottom/rgb-u16-op2.xcf", "afaf37e8e05d3c46ebacb7ed2f7de94549c671ccfc1ad28f0509c6a49bd69990", 8032, 2),
    ("normal-mask-geometry-integer-evidence/right-bottom/rgb-u16-op2.edit.xcf", "188a07be225a4ba6089a12426518fd623bcc31e83f3649584903b09181bfaf72", 8032, 2),
    ("normal-mask-geometry-integer-evidence/right-bottom/rgb-u16-op3.xcf", "226bbaccd38621c1281226ea3d8cd0ced481dc4c00706cb64a4eb2cd41d2582e", 8032, 3),
    ("normal-mask-geometry-integer-evidence/right-bottom/rgb-u16-op3.edit.xcf", "d924f2f613d20f64daa178cb4ba8b1877e523fdd65c563c3d418a6204fcc64cf", 8032, 3),
    ("normal-mask-geometry-integer-evidence/right-bottom/rgb-u16-op4.xcf", "e77a765799d65e777f7a23ac95097e1ffc045d676adc918a0fc9fb1d4821d4eb", 8032, 4),
    ("normal-mask-geometry-integer-evidence/right-bottom/rgb-u16-op4.edit.xcf", "66ffaf8f400403d93c1c5e005bf6db9b298d1e8e051ba7edc0799c961ae9191a", 8032, 4),
    ("normal-mask-geometry-integer-evidence/right-bottom/rgb-u32-op1.xcf", "ec767073f9e1b1d3d0c5242af22c778cb1ba395d65687d7c81407dff74111ddd", 13722, 1),
    ("normal-mask-geometry-integer-evidence/right-bottom/rgb-u32-op1.edit.xcf", "f0ee58b09ad17524279957b947dafde4077b828f54e05f0c606ec83935f99a36", 13722, 1),
    ("normal-mask-geometry-integer-evidence/right-bottom/rgb-u32-op2.xcf", "279c70d5eb892acb874f1aef3510b53c0d94ccc4b0fd1deedd355f83a279f0b7", 13722, 2),
    ("normal-mask-geometry-integer-evidence/right-bottom/rgb-u32-op2.edit.xcf", "f3104e9c51de3c1b78ffa2cea3c9aec82456e7b03273890be9d137734b00d3fd", 13722, 2),
    ("normal-mask-geometry-integer-evidence/right-bottom/rgb-u32-op3.xcf", "6a37b2aabcd416793ad7eedb4a580c25b028e6190c1f913b779ca92bb75433a1", 13722, 3),
    ("normal-mask-geometry-integer-evidence/right-bottom/rgb-u32-op3.edit.xcf", "68257a68ccf835639fc51a2f081512f4738bf28f79708d3ef9babdb47096704b", 13722, 3),
    ("normal-mask-geometry-integer-evidence/right-bottom/rgb-u32-op4.xcf", "7956d583b81dec8e3aee09ca382c99d04cfb1f7c3edefcc8a68e084bb844bb87", 13722, 4),
    ("normal-mask-geometry-integer-evidence/right-bottom/rgb-u32-op4.edit.xcf", "a9015296e2f2d42dda961282dbf7c388d53c4362a6e44b96516621e242c5971b", 13722, 4),
    ("normal-mask-geometry-integer-evidence/right-bottom/gray-u16-op1.xcf", "10e91bcf488942ff308d5ff96ce3f7bc3b32e307f08dd21d253b0a23029c7d4d", 5076, 1),
    ("normal-mask-geometry-integer-evidence/right-bottom/gray-u16-op1.edit.xcf", "8d1f60b922fb1c582c9281dd6c76d7ebdd2fd1f144bf56b8300933a237fe9204", 5076, 1),
    ("normal-mask-geometry-integer-evidence/right-bottom/gray-u16-op2.xcf", "a9ed7cc56ca13c19aa335b9e70f1b657c00c3514d03453c5733f38547c139f84", 5076, 2),
    ("normal-mask-geometry-integer-evidence/right-bottom/gray-u16-op2.edit.xcf", "94eb42c878feacdf01d45b96fb5b1c54300ab878a774bd1e5effb067a4187c05", 5076, 2),
    ("normal-mask-geometry-integer-evidence/right-bottom/gray-u16-op3.xcf", "ebe072ad3f1a7ff306d4324cdadf677ac49417f6a2bdd882b10e0560e2bb9fe4", 5076, 3),
    ("normal-mask-geometry-integer-evidence/right-bottom/gray-u16-op3.edit.xcf", "c4c5c0b6923dbdbaf62107a9ded51ef39699aca8a42265b1cb9408f69f367f67", 5076, 3),
    ("normal-mask-geometry-integer-evidence/right-bottom/gray-u16-op4.xcf", "646b2e5656fe9a34b331528dfc66b01e1adcb40a197e73312a815be0cac0f6e4", 5076, 4),
    ("normal-mask-geometry-integer-evidence/right-bottom/gray-u16-op4.edit.xcf", "52b864e9524df02fbd306ca539cfd455024cd644d60c4ee7e94324a662e04489", 5076, 4),
    ("normal-mask-geometry-integer-evidence/right-bottom/gray-u32-op1.xcf", "0649a852c01bf152bf5e7f384fd64896cec4394eaca794c977d8142d168d80ab", 8014, 1),
    ("normal-mask-geometry-integer-evidence/right-bottom/gray-u32-op1.edit.xcf", "3b90d5e5a4cbafa30113b66d337cae01aa30c22f940c415c11bf9e922a43ad41", 8014, 1),
    ("normal-mask-geometry-integer-evidence/right-bottom/gray-u32-op2.xcf", "3ae2df55eb9b214bb761e6d040fbe6cee20cb5db3cec0e04433791dbe5aac03f", 8014, 2),
    ("normal-mask-geometry-integer-evidence/right-bottom/gray-u32-op2.edit.xcf", "4a875eceaf76236d98d65efbeee4517855aaf755e45285f5db04907158dc861b", 8014, 2),
    ("normal-mask-geometry-integer-evidence/right-bottom/gray-u32-op3.xcf", "fa7c2ddceab5e6560aaf78f2af4f6a3561aced44247430b83b4c5a4e9c50ce29", 8014, 3),
    ("normal-mask-geometry-integer-evidence/right-bottom/gray-u32-op3.edit.xcf", "964bdb89b08403250c3f991c62bf2f07db3c714ca6c692887d264a5d7ea4b671", 8014, 3),
    ("normal-mask-geometry-integer-evidence/right-bottom/gray-u32-op4.xcf", "27d18358ef333ce7f0831b06db95a38bbe1f2bcf2bfe95c9229f10f7d7b4b601", 8014, 4),
    ("normal-mask-geometry-integer-evidence/right-bottom/gray-u32-op4.edit.xcf", "c68e8185d6b0ecb0506bda4cfe1c9bf7268ca6271946185ef3285a4b007f45a5", 8014, 4),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f32-op1.xcf", "ff29148ffec4a0c845441d44d26da3aa666e277a77731faf16285cfc69923974", 9339, 1),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f32-op1.edit.xcf", "e9ecc3d9df4e1f16276e78c0785dedebf0686b232818d03319631b5cbd3aca2c", 9339, 1),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f32-op2.xcf", "0d96b8736f3d71b547f062a3aaf6852fbbe035f182b3b0af745f413abba6ba47", 9339, 2),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f32-op2.edit.xcf", "55e987014a29f602a8e140b784f42cf0555c16f07d5134b33300942e573abef9", 9339, 2),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f32-op3.xcf", "ed64ad0823b55057b91db1df92b7f2949fbe21ebf06db219c407f6efad6ddf9c", 9339, 3),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f32-op3.edit.xcf", "97bc281529e440b779c16ca95e6e160cb973197361764ffa938372fb6c7fe621", 9339, 3),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f32-op4.xcf", "8132f936a8d8cf2051f9904e7f5bc99e5aaad75de2114baa23d61db5dae84d70", 9339, 4),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f32-op4.edit.xcf", "e5055af75cbd4f8a2e6553af542b4a2096fcc88ca70a2582314ee25cb4ed0dd1", 9339, 4),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f64-op1.xcf", "45a731c7dec5f4004ce4ce95cc08a5d7158643f5f92df45fbb7f8a22565277a8", 13710, 1),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f64-op1.edit.xcf", "6d61f9f84f4a2fcd21cfafa22f7f08f3e931cdcea3a0842895762a3b920bb90f", 13706, 1),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f64-op2.xcf", "0f58ccb432193ad8d8b2e598a85edcd0d786bdeec38ae2ef4fc55b085bee25d3", 13710, 2),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f64-op2.edit.xcf", "bb1ab9cfc0ff5906ba0c7961c814d066b296552cbe4d6f4b9291f9bdbd01fdf5", 13706, 2),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f64-op3.xcf", "ea80f910a08de501fb969803d8fdc348bd73d971e62969e7bdc2dc0dbaae2477", 13710, 3),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f64-op3.edit.xcf", "fabe0ab3f8a588948e424e8adbb06f59d690f6aa7674543e10ad581f9511b943", 13706, 3),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f64-op4.xcf", "707c317381ab0fc1b390beb5f1e377e0a3c2d61bf0f6ec99eec1f98cef01f02f", 13710, 4),
    ("normal-mask-geometry-linear-evidence/left-top/rgb-f64-op4.edit.xcf", "a123b34fe1edb88d1f4a7e92ce751ee83555dde5c583aee39e32459551127e37", 13706, 4),
    ("normal-mask-geometry-linear-evidence/left-top/gray-f32-op1.xcf", "2e67b171d0b5d46ca601721611af65287fd897d82c19b74d09038cd647baebba", 5247, 1),
    ("normal-mask-geometry-linear-evidence/left-top/gray-f32-op1.edit.xcf", "58222ceee401e0bf50768065330d16ab8eb4d027a835ad69e297df6b7e6a9831", 5247, 1),
    ("normal-mask-geometry-linear-evidence/left-top/gray-f32-op2.xcf", "3da1d9cad5307c60059779186028a77626b8194a8e139c601a86d06448c3053a", 5247, 2),
    ("normal-mask-geometry-linear-evidence/left-top/gray-f32-op2.edit.xcf", "e0cdeee1fbf204c77fbf8eaa882324de439240118679087d70feb6aed97dcf62", 5247, 2),
    ("normal-mask-geometry-linear-evidence/left-top/gray-f32-op3.xcf", "5fa48ea2f673977f960d8ae5f43f77beeeab821c6648929d1e469695da765f34", 5247, 3),
    ("normal-mask-geometry-linear-evidence/left-top/gray-f32-op3.edit.xcf", "44cca1f3a8ac3ff5f1f045e9319949e84bbb70763cf32379c2c99a129c22bf24", 5247, 3),
    ("normal-mask-geometry-linear-evidence/left-top/gray-f32-op4.xcf", "3f84c611ed7cf7e2e4556c047c2438d104330f38f4e3cafe36eeb9a45508b8fb", 5247, 4),
    ("normal-mask-geometry-linear-evidence/left-top/gray-f32-op4.edit.xcf", "2742b38925acdcf32e5bff6640971d9d1a10e7d574d9787d50088c1622a04c37", 5247, 4),
    ("normal-mask-geometry-linear-evidence/left-top/gray-f64-op1.xcf", "fc144090592f149c139cc6f980acdd03664746a4e17568ad64f814a4a0299468", 6690, 1),
    ("normal-mask-geometry-linear-evidence/left-top/gray-f64-op1.edit.xcf", "19127848cfd4e6c0311e4a34bfe436c3e36f38804d76331eb2e09422be0aa3fb", 6686, 1),
    ("normal-mask-geometry-linear-evidence/left-top/gray-f64-op2.xcf", "dee52b5be4b63f26a100af6413cf970836a100e55866092158b7d8dbdb72c361", 6690, 2),
    ("normal-mask-geometry-linear-evidence/left-top/gray-f64-op2.edit.xcf", "335d661af97d2508046c8d72e838e65a16aa4cda8c67a3003a5ecc87596dcaa8", 6686, 2),
    ("normal-mask-geometry-linear-evidence/left-top/gray-f64-op3.xcf", "cdbdd0080b0451399d64cfa80427ddd3ab4518fb6f38ff27f8e5940d7f52661d", 6690, 3),
    ("normal-mask-geometry-linear-evidence/left-top/gray-f64-op3.edit.xcf", "e6289de51cdcb8f1697a1229936b78a6ad30965e0bb2d81ec49a5fa223521b19", 6686, 3),
    ("normal-mask-geometry-linear-evidence/left-top/gray-f64-op4.xcf", "8e43e6815c87bac3374b2aeaf1905d6da8a242d04878e5341d68587800d27fc5", 6690, 4),
    ("normal-mask-geometry-linear-evidence/left-top/gray-f64-op4.edit.xcf", "b277af9b5987fda5d803a5a79dbdf1ccf51eb6226b2f19d1450ef1a83e898be6", 6686, 4),
    ("normal-mask-geometry-linear-evidence/right-bottom/rgb-f32-op1.xcf", "0bdd36bfb4fbd79702758938c93cb5621646f54433a505e7cb9ceae39037e24b", 12177, 1),
    ("normal-mask-geometry-linear-evidence/right-bottom/rgb-f32-op1.edit.xcf", "4ce316ac4c9c05189e1accd85bb513a864769805567293dfaa199e3acf883007", 12177, 1),
    ("normal-mask-geometry-linear-evidence/right-bottom/rgb-f32-op2.xcf", "c35d9fa840556154d69227a5844565c58dbedd1ba76f0a94f2186d21c5988cec", 12177, 2),
    ("normal-mask-geometry-linear-evidence/right-bottom/rgb-f32-op2.edit.xcf", "dff2d219e3311975101b84e67cf67b45276a3363944571fd38387c2e7ad34765", 12177, 2),
    ("normal-mask-geometry-linear-evidence/right-bottom/rgb-f32-op3.xcf", "b544cfe07b0b9978ff502d4615cfd47881ca21c9f1ca3c1e6b9353759b2e6a23", 12177, 3),
    ("normal-mask-geometry-linear-evidence/right-bottom/rgb-f32-op3.edit.xcf", "80153fee22d1733242b8a5900ac8cfe55550c0c5de4dc0656bb3de61faa4f561", 12177, 3),
    ("normal-mask-geometry-linear-evidence/right-bottom/rgb-f32-op4.xcf", "1d35aa900395dd77ccba3b30913075d872233e7a33391f088f271760f9581957", 12177, 4),
    ("normal-mask-geometry-linear-evidence/right-bottom/rgb-f32-op4.edit.xcf", "cb428fc21f6781bd9a4faafd144d7c931019ba4d6098512164c07a1b294e6318", 12177, 4),
    ("normal-mask-geometry-linear-evidence/right-bottom/rgb-f64-op1.xcf", "b1504db52229023efe1cc8882d50a01221c49adcbd23b639b7df7a17b594f236", 18326, 1),
    ("normal-mask-geometry-linear-evidence/right-bottom/rgb-f64-op1.edit.xcf", "196760af08843723e30ad1f9777c7b93b0f0c92ea0a9551afc0507aabd8e1601", 18327, 1),
    ("normal-mask-geometry-linear-evidence/right-bottom/rgb-f64-op2.xcf", "d23abe2e031b6d6d9e6bcf8fa73b30597b6f36ce800a07e9260a9eeb5552c5f6", 18326, 2),
    ("normal-mask-geometry-linear-evidence/right-bottom/rgb-f64-op2.edit.xcf", "768c9a82b4ddf0e238f75057e2b19520e7ee9876987b826f85e6648b236c4164", 18327, 2),
    ("normal-mask-geometry-linear-evidence/right-bottom/rgb-f64-op3.xcf", "559431cc50b6d37754bbf34854a62f5044e07e24386944215b23a21132fba258", 18326, 3),
    ("normal-mask-geometry-linear-evidence/right-bottom/rgb-f64-op3.edit.xcf", "1293d0a0068dcd25234bfee765a3fbb4fce6d768383053d59cc5a6aa834e2f59", 18327, 3),
    ("normal-mask-geometry-linear-evidence/right-bottom/rgb-f64-op4.xcf", "da356fc66288972a63ec279cf7f01df0d37abb0ed28c4bb3d279f10604e0b579", 18326, 4),
    ("normal-mask-geometry-linear-evidence/right-bottom/rgb-f64-op4.edit.xcf", "447f7aa89c3c137b46073ca07fb4792c06648aaa1b0e06184e0d59eb41e0e797", 18327, 4),
    ("normal-mask-geometry-linear-evidence/right-bottom/gray-f32-op1.xcf", "810c8fde459b5a4030350f9e4954ea07d6e25eebd12e7e4086fc835b2fa993ef", 6469, 1),
    ("normal-mask-geometry-linear-evidence/right-bottom/gray-f32-op1.edit.xcf", "ea023f69a927e2b37ce235313eae79bf2aabaa96fcd54f2eb9447a7f1ed027d6", 6469, 1),
    ("normal-mask-geometry-linear-evidence/right-bottom/gray-f32-op2.xcf", "c9e46a40152d2cb0c36b536efd0ceac0f42babfa76012167bafe1719b257b86b", 6469, 2),
    ("normal-mask-geometry-linear-evidence/right-bottom/gray-f32-op2.edit.xcf", "2a9874bdb25fe3680a12c6328fea2bcda4a9a94a5a547280b5a50cb565160349", 6469, 2),
    ("normal-mask-geometry-linear-evidence/right-bottom/gray-f32-op3.xcf", "a0c35f32c367dbce4dfe4164bbc979cc045181e3aae9630dd124e0434230bcb3", 6469, 3),
    ("normal-mask-geometry-linear-evidence/right-bottom/gray-f32-op3.edit.xcf", "2f59fd758b72cb743caa44e0c444773020e6f4b0cb4081e48a09bff78afa67ee", 6469, 3),
    ("normal-mask-geometry-linear-evidence/right-bottom/gray-f32-op4.xcf", "0a00102e567ecc9282328165cbc515ac5a8eb53176c2d3085b1ad1a95dff759e", 6469, 4),
    ("normal-mask-geometry-linear-evidence/right-bottom/gray-f32-op4.edit.xcf", "7155c496b6d561faceab916837ec40e5d3d86ece54dffab7baa266b2a709279a", 6469, 4),
    ("normal-mask-geometry-linear-evidence/right-bottom/gray-f64-op1.xcf", "1b1f47f94420ad05c0e627f818feb822502ee4ad74eba2a57973d40ea074dd33", 8478, 1),
    ("normal-mask-geometry-linear-evidence/right-bottom/gray-f64-op1.edit.xcf", "8e9fd0cceba626f643269525343f0eabc8641ed0ac4cdce27f891fe283841ce9", 8479, 1),
    ("normal-mask-geometry-linear-evidence/right-bottom/gray-f64-op2.xcf", "02c5594853ad61835f7905d3d261ee744d218041e210cdf2d84fe1a5ca700947", 8478, 2),
    ("normal-mask-geometry-linear-evidence/right-bottom/gray-f64-op2.edit.xcf", "f0f99b497f72d6a8c4a839a9a09c842cccbc0e1e3cb6f1a80857833ba1ef50cd", 8479, 2),
    ("normal-mask-geometry-linear-evidence/right-bottom/gray-f64-op3.xcf", "7c5f4125f21769f724682473e622d6798a3f66a068c72f50c01c874b6402d4e3", 8478, 3),
    ("normal-mask-geometry-linear-evidence/right-bottom/gray-f64-op3.edit.xcf", "4ab7c545cd159fb11d896e8052b613598e627b5beefbaadfe9980396e5820fce", 8479, 3),
    ("normal-mask-geometry-linear-evidence/right-bottom/gray-f64-op4.xcf", "1a78abf8bb0ff78b221928c882606fc6cdde11f80c24cb7b697f1596c4325e41", 8478, 4),
    ("normal-mask-geometry-linear-evidence/right-bottom/gray-f64-op4.edit.xcf", "13b2646293fa0454c1275b302b6d6719f1627f428768d6f8238669402e994484", 8479, 4),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f32-op1.xcf", "3451e9edfcf702864dc8529a48bbd5ec5ac12c5472998d79a1537cdeeb381790", 9339, 1),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f32-op1.edit.xcf", "cceca9a59fc1f108e6e9145270a162530f2091c2d5d5518bfcddd1692b59f79f", 9339, 1),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f32-op2.xcf", "69e7957c4b6e2b7351c9605519cae1d2dfc0f4c00c8456d850d7a3db7cf942b0", 9339, 2),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f32-op2.edit.xcf", "7c894988391ea779e748790c7a3d30c60cb439a633f2a499a3b54dcb2c00c715", 9339, 2),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f32-op3.xcf", "0e78ed8b0bd8a4803c18f8d2ec2c775711a523aabd53c458d40f873a4eca23d2", 9339, 3),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f32-op3.edit.xcf", "e3ff829e102295e00a901b464f3e11417534584dedd6caa7ca82d8bba5356b53", 9339, 3),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f32-op4.xcf", "0a825021886e1d2f7b064e01bbd20c0581ff522178e9ce7744fce66ee33c44db", 9339, 4),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f32-op4.edit.xcf", "5f5f0c2c120fc4306155dc3466c3983520aeeb1ea6326d95dffb5ed649579ff3", 9339, 4),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f64-op1.xcf", "a811e5a2f8f3e46251479b0ae5ea855fa84b6b7d14c9c47da6ed1e1969ba75ec", 13710, 1),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f64-op1.edit.xcf", "c54c497f08b540a86d2e848763cdd0d6cca11ed8d36633b39965f63e7b31e63e", 13706, 1),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f64-op2.xcf", "2d886b9e924c57f0ee05c1ab35e6cceac13eae0f90f6017f47149d34b353aad0", 13710, 2),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f64-op2.edit.xcf", "6d522b820d94feb09bfaa2169c74f2beb84e6e384ffe98003d3744e649fd256f", 13706, 2),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f64-op3.xcf", "d3ded016dc359693fce24312db211df66bcdac6442998349205d578e2bbcaa29", 13710, 3),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f64-op3.edit.xcf", "ef6839de34d41c57fb654b7f563a1b581c1594d6653183d65ed49ec8fac52214", 13706, 3),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f64-op4.xcf", "ffa5d4e0152466684f3f6dec172b4005c2c484a7a2a0a45956f88377602f7ef7", 13710, 4),
    ("normal-mask-geometry-perceptual-evidence/left-top/rgb-f64-op4.edit.xcf", "77faa8587fa49d8c97e440926c1cc63ea59447b302bda9128605900eb28a1c02", 13706, 4),
    ("normal-mask-geometry-perceptual-evidence/left-top/gray-f32-op1.xcf", "826f043bb684df4b353d28ad9d19ddb488c8984b4569099b856ddf2c7cfc4251", 5247, 1),
    ("normal-mask-geometry-perceptual-evidence/left-top/gray-f32-op1.edit.xcf", "a01e88a62507b2e291a237bd19f692fdcd538ded5efbfbf30fa09c77ade69fa9", 5247, 1),
    ("normal-mask-geometry-perceptual-evidence/left-top/gray-f32-op2.xcf", "2cd715306242b1be2701057706d1426b9ff2a3425495913f9b92b57cf70a81ef", 5247, 2),
    ("normal-mask-geometry-perceptual-evidence/left-top/gray-f32-op2.edit.xcf", "3bc874939ddc3297ddb575bf35d8fdf39f409467477433f0f6ca0b205b4e8d1c", 5247, 2),
    ("normal-mask-geometry-perceptual-evidence/left-top/gray-f32-op3.xcf", "be32290b027725c05f76ad4a654e25c5bc75fac66e82efe92a2b4bb168be1809", 5247, 3),
    ("normal-mask-geometry-perceptual-evidence/left-top/gray-f32-op3.edit.xcf", "a3ccf96ba949be847a2ca25d4013015f24718ea0c28215ca49fab7cc5d63500d", 5247, 3),
    ("normal-mask-geometry-perceptual-evidence/left-top/gray-f32-op4.xcf", "bd6c03a429ccedb8c4afcad4c12563184eb1ab776c58a37028f9c7d2c691dcd9", 5247, 4),
    ("normal-mask-geometry-perceptual-evidence/left-top/gray-f32-op4.edit.xcf", "278a1278b28b94ef391eabb8899ec86746a60b85c2aa4f2041df0a5ccb1f4aad", 5247, 4),
    ("normal-mask-geometry-perceptual-evidence/left-top/gray-f64-op1.xcf", "d6882a35bf13dca9366e57370afa502ee271eaf7ff3adb24c9c53bf163c6963e", 6690, 1),
    ("normal-mask-geometry-perceptual-evidence/left-top/gray-f64-op1.edit.xcf", "045de31c546e062c798800f8fa1c2163ac028ad6f09ccc9a439038929f53f4f5", 6686, 1),
    ("normal-mask-geometry-perceptual-evidence/left-top/gray-f64-op2.xcf", "bac3fdca4ca0bd427fdf39ed3549788c0b74e3c70298b460c981fe120736e05c", 6690, 2),
    ("normal-mask-geometry-perceptual-evidence/left-top/gray-f64-op2.edit.xcf", "fe47d4e9a09ef421c814da0def45fef7ee8d31125fd3e89624e7ca006385e544", 6686, 2),
    ("normal-mask-geometry-perceptual-evidence/left-top/gray-f64-op3.xcf", "106040db4a6de949c0116cd625bd706734d118721184a4bea8d38e94c2d44eb1", 6690, 3),
    ("normal-mask-geometry-perceptual-evidence/left-top/gray-f64-op3.edit.xcf", "f41367952178c3c4b6efcefc87f4ce9bcc16051d19b65d2970b867020fb49f4c", 6686, 3),
    ("normal-mask-geometry-perceptual-evidence/left-top/gray-f64-op4.xcf", "885b21f43b3461074eeb34f77e8509ec0a415a95f6e98fff6e77677317b71a00", 6690, 4),
    ("normal-mask-geometry-perceptual-evidence/left-top/gray-f64-op4.edit.xcf", "0c676034d511cc075c6e9eacf7aadb124d0c4d3d43ad3c2fabb3ba0243850752", 6686, 4),
    ("normal-mask-geometry-perceptual-evidence/right-bottom/rgb-f32-op1.xcf", "8d1226c0e98e8dc6627b6230693299423cf999d9ba3efe9a67fbe8ce11e63264", 12177, 1),
    (
        "normal-mask-geometry-perceptual-evidence/right-bottom/rgb-f32-op1.edit.xcf",
        "0dda201db95c70eacd12a9e602f7a67d20376d36c5c89544827ac391d3e89fec",
        12177,
        1,
    ),
    ("normal-mask-geometry-perceptual-evidence/right-bottom/rgb-f32-op2.xcf", "562d610f11e03fb57dac9534f21d5025bb140023c9ba913e41f8b7bf306593e4", 12177, 2),
    (
        "normal-mask-geometry-perceptual-evidence/right-bottom/rgb-f32-op2.edit.xcf",
        "4f116524a8de3ccfa49eef1c0047249a75c1fd76a722e837a54e7c511d2deaf1",
        12177,
        2,
    ),
    ("normal-mask-geometry-perceptual-evidence/right-bottom/rgb-f32-op3.xcf", "fc0cd4d1291d8678c452d26c2e2d0e3f478e9dd569fb236da1ad6aa7042fb593", 12177, 3),
    (
        "normal-mask-geometry-perceptual-evidence/right-bottom/rgb-f32-op3.edit.xcf",
        "326f677128a53731ca98239a3d2411bb2dee9cf4ec52001c9f323f1393a816c1",
        12177,
        3,
    ),
    ("normal-mask-geometry-perceptual-evidence/right-bottom/rgb-f32-op4.xcf", "80be9114d41189661abe68d6691877044f48a790811b89bdaea059943ed69e5c", 12177, 4),
    (
        "normal-mask-geometry-perceptual-evidence/right-bottom/rgb-f32-op4.edit.xcf",
        "09d9626ec4ec7d205c044440cce2be52b8916d5ff60d099b70acaa98d96cc9b4",
        12177,
        4,
    ),
    ("normal-mask-geometry-perceptual-evidence/right-bottom/rgb-f64-op1.xcf", "286000e291b0cb98a4e05abab0c4fe9cd1b46103894c41c98799f6835e799a29", 18326, 1),
    (
        "normal-mask-geometry-perceptual-evidence/right-bottom/rgb-f64-op1.edit.xcf",
        "5a0ccdc319a39aa69a2768356ad04038472855effaec7ab6b4d552a66cf29cfd",
        18327,
        1,
    ),
    ("normal-mask-geometry-perceptual-evidence/right-bottom/rgb-f64-op2.xcf", "a7cf1ac07805ac234517349b4e9c9bc8189ce609f5e83d946e972e4a9ce58e25", 18326, 2),
    (
        "normal-mask-geometry-perceptual-evidence/right-bottom/rgb-f64-op2.edit.xcf",
        "7aae88960e3759ec825018bd11e9dbe5d426d1d8e0f8322eb043a1bbed728b1a",
        18327,
        2,
    ),
    ("normal-mask-geometry-perceptual-evidence/right-bottom/rgb-f64-op3.xcf", "97116ff13f2d3344b248641942ae4b224e2dd60fccf30c10d8ac8b4462ba5ec8", 18326, 3),
    (
        "normal-mask-geometry-perceptual-evidence/right-bottom/rgb-f64-op3.edit.xcf",
        "b294fde7e9a5f7b4a0dc4ac87cce1d43432167aac3c9a302aeeda941dedc32ff",
        18327,
        3,
    ),
    ("normal-mask-geometry-perceptual-evidence/right-bottom/rgb-f64-op4.xcf", "29f44827c2d8bb6fc46b1729417a3e28667203cac1d73da3378905874d6523ab", 18326, 4),
    (
        "normal-mask-geometry-perceptual-evidence/right-bottom/rgb-f64-op4.edit.xcf",
        "e0e811f179ad5202411117c3359f7e7164308be672744ec8c4c2d86519d868fd",
        18327,
        4,
    ),
    ("normal-mask-geometry-perceptual-evidence/right-bottom/gray-f32-op1.xcf", "39a7e533c664dc18899ee21f5beff4b545960f4fe70e1b692532f9e27b4ad48d", 6469, 1),
    (
        "normal-mask-geometry-perceptual-evidence/right-bottom/gray-f32-op1.edit.xcf",
        "937307a868319e3360554f1c78b744e976d11aca8bd3c2f295e6c04dcc6f9160",
        6469,
        1,
    ),
    ("normal-mask-geometry-perceptual-evidence/right-bottom/gray-f32-op2.xcf", "034987788d415691860434f6139514d5de5bd516cb1cf2922124346e70645380", 6469, 2),
    (
        "normal-mask-geometry-perceptual-evidence/right-bottom/gray-f32-op2.edit.xcf",
        "0fb68efa46b4c04062c44d56a8cf4d6af14a966e7f97445e3a88ea71a95e9a82",
        6469,
        2,
    ),
    ("normal-mask-geometry-perceptual-evidence/right-bottom/gray-f32-op3.xcf", "c237520c8f705b6e9ad5e1a253e9fcee3ecb630507a31c73539e574f688a8da1", 6469, 3),
    (
        "normal-mask-geometry-perceptual-evidence/right-bottom/gray-f32-op3.edit.xcf",
        "6e55f1875e6734ee35729ad12c2a0d6ea3d24d494917a68c751324c72554fbf0",
        6469,
        3,
    ),
    ("normal-mask-geometry-perceptual-evidence/right-bottom/gray-f32-op4.xcf", "4ff0c0898ac6650f3338972d91b35d225257801e923e487c702fb600f6a20c09", 6469, 4),
    (
        "normal-mask-geometry-perceptual-evidence/right-bottom/gray-f32-op4.edit.xcf",
        "5f0b45cb8d9beef72c8080dec4d6f50c6331d3bbeed551e230935ee9684f0c4a",
        6469,
        4,
    ),
    ("normal-mask-geometry-perceptual-evidence/right-bottom/gray-f64-op1.xcf", "05093aceeaf301e7637a9b3d4c11d98c1705f9c67050ccfff0aeae6d2e9de45e", 8478, 1),
    (
        "normal-mask-geometry-perceptual-evidence/right-bottom/gray-f64-op1.edit.xcf",
        "d890ebc5275e00da4fadd3faaea86cce679d0e604af557c424d32d2aee0e9625",
        8479,
        1,
    ),
    ("normal-mask-geometry-perceptual-evidence/right-bottom/gray-f64-op2.xcf", "4757d99d3c3d584922cef4b9270664825f3d85bad297584a8507300c296857ea", 8478, 2),
    (
        "normal-mask-geometry-perceptual-evidence/right-bottom/gray-f64-op2.edit.xcf",
        "7e93c2fd364d367a83e3150cd350d1a649fda128d7ad70241ecaac48e602769b",
        8479,
        2,
    ),
    ("normal-mask-geometry-perceptual-evidence/right-bottom/gray-f64-op3.xcf", "1c3c62c7cd1b79703d89e54efd8bd72a909371642c5cd4ddaadb1c9304038b39", 8478, 3),
    (
        "normal-mask-geometry-perceptual-evidence/right-bottom/gray-f64-op3.edit.xcf",
        "a76319346003a7cf4fb34b49aaadb058a032266331d8dea939fcc2d3cbb36408",
        8479,
        3,
    ),
    ("normal-mask-geometry-perceptual-evidence/right-bottom/gray-f64-op4.xcf", "428383aa0e021ed942e3f0a15cd41f860f188cd1d7bc03aec96322c56aa20f92", 8478, 4),
    (
        "normal-mask-geometry-perceptual-evidence/right-bottom/gray-f64-op4.edit.xcf",
        "27d09eb74a98eaed1c9f329874f569f09ecf22c394475c85d3cecdec564a0405",
        8479,
        4,
    ),
];
const PROOFS: [(&str, &str); 8] = [
    ("normal-mask-geometry-holdout-evidence/left-top/observations.json", "1a565ade2d3f9f3dcef51124cda6b46d656e7785c0958a6bed75043b85f01633"),
    ("normal-mask-geometry-holdout-evidence/right-bottom/observations.json", "52b4a0906fdc09d4f84e264bd9cff45bb6b53457cdb7808248599f5d007c557e"),
    ("normal-mask-geometry-integer-evidence/left-top/observations.json", "b55d75f1fe4b8fe47ff7eef87e2806bbf4d4c27401c392411f16297d29673884"),
    ("normal-mask-geometry-integer-evidence/right-bottom/observations.json", "1044b554b418b68f213e7c04ba055896255a937c55fd01fbcc114922ed039d4f"),
    ("normal-mask-geometry-linear-evidence/left-top/observations.json", "80d017fc2897fd449b8386c98e73f1d6caaded57f74cd0aaec6d2eddb80822cc"),
    ("normal-mask-geometry-linear-evidence/right-bottom/observations.json", "a4c12094204696a243d8136ebbda762681fe02ae68cea984b2beae1950364566"),
    ("normal-mask-geometry-perceptual-evidence/left-top/observations.json", "6cd4ad61905af74728ced7d2b13e8ea47c16ffc64d4de9cedc6a24f3a9478abc"),
    ("normal-mask-geometry-perceptual-evidence/right-bottom/observations.json", "4618d2efdf9c21196634ebd60cc9a92369f9ae0cebe4d17469282c68f7bce1c2"),
];

fn root() -> PathBuf {
    PathBuf::from(std::env::var_os("PHOTOCRAFT_XCF_FULL_CORPUS").unwrap())
}
fn source(path: &str, pin: &str, len: usize) -> Vec<u8> {
    let b = std::fs::read(root().join(path)).unwrap();
    assert_eq!(b.len(), len);
    assert_eq!(hash(&b), pin);
    b
}
fn imported(path: &str, bytes: &[u8], q: &BackingQuota) -> photocraft_io::OwnedAnalyticGeometryXcfImport {
    let limits = photocraft_xcf::ResourceLimits::for_policy(Default::default())
        .unwrap()
        .lowered(&[("residentBytes", 64 << 20), ("indexBytes", 8 << 20), ("scratchBytes", 0)])
        .unwrap();
    let result = photocraft_io::import_owned_xcf_from_reader_in_quota(
        path,
        &mut Cursor::new(bytes),
        bytes.len() as u64,
        &Default::default(),
        &limits,
        q.clone(),
        &ctl(),
    )
    .unwrap();
    let photocraft_io::OwnedXcfImport::AnalyticGeometry(owner) = result else { panic!("exact original analytic geometry route without fallback") };
    owner
}
fn native(s: &Session) -> (Vec<NativePlaneDescriptor>, String) {
    let doc = &s.active().unwrap().doc;
    let mut descriptors = Vec::new();
    let mut h = Sha256::new();
    for layer in &doc.layers {
        let LayerContent::BackedRaster(backed) = &layer.content else { panic!("actual native owner") };
        descriptors.push(backed.descriptor());
        h.update(backed.descriptor().default_bytes());
        for coord in backed.descriptor().bounds().tiles() {
            let p = backed.checked_view().pin_tile(coord, &ctl()).unwrap();
            match p.encoded() {
                NativeTileBytes::FullTile(b) | NativeTileBytes::DefaultPixel(b) => h.update(b),
            }
        }
        if let Some(mask) = &layer.mask {
            h.update(mask.surface.default_bytes());
            for (_, tile) in mask.surface.tiles() {
                h.update(tile.bytes());
            }
        }
    }
    (descriptors, h.finalize().iter().map(|v| format!("{v:02x}")).collect())
}
fn archive(bytes: &[u8], q: &BackingQuota) -> OwnedDocumentArchive {
    photocraft_io::accounted_native::open_accounted_native_from_reader(&mut Cursor::new(bytes), bytes.len() as u64, Default::default(), q.clone(), &ctl())
        .unwrap()
}
#[test]
#[ignore = "UNRUN: 256 pinned original/edit NL/L/P four-operator geometries, typed source/fullcurrent validation, paid SaveAs and strict native reopen"]
fn analytic_source_saveas_and_native_reopen_keep_exact_mask_profile_geometry_and_source_guard() {
    for (path, pin) in PROOFS {
        assert_eq!(hash(&std::fs::read(root().join(path)).unwrap()), pin);
    }
    for (path, pin, len, operator) in CASES {
        let bytes = source(path, pin, len);
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
        let owner = imported(path, &bytes, &q);
        let mut s = Session::new();
        s.add_owned_analytic_geometry_xcf_import(&owner, path, Some(path), &ctl()).unwrap();
        drop(owner);
        let initial = s.active().unwrap().doc.clone();
        let expected_operator = match operator {
            1 => photocraft_doc::import_compositing::GimpOperator::Union,
            2 => photocraft_doc::import_compositing::GimpOperator::ClipToBackdrop,
            3 => photocraft_doc::import_compositing::GimpOperator::ClipToLayer,
            4 => photocraft_doc::import_compositing::GimpOperator::Intersection,
            _ => panic!("pinned fixture operator"),
        };
        let foreground = initial.layers.last().unwrap();
        let policy = initial
            .xcf_state
            .as_ref()
            .unwrap()
            .items
            .iter()
            .find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(foreground.id))
            .unwrap()
            .composition
            .as_ref()
            .unwrap();
        assert_eq!(policy.stored_operator, operator);
        assert_eq!(policy.resolved.as_ref().unwrap().operator, expected_operator);
        let encoding = if path.contains("-linear-evidence/") {
            photocraft_doc::SampleEncoding::Linear
        } else if path.contains("-perceptual-evidence/") {
            photocraft_doc::SampleEncoding::Perceptual
        } else {
            photocraft_doc::SampleEncoding::NonlinearProfile
        };
        assert_eq!(initial.sample_encoding, Some(encoding));
        let raw = native(&s);
        assert!(is_current_analytic_geometry_candidate(&initial, &ctl()).unwrap());
        assert!(!initial.accounted().unwrap().is_readonly_point_storage());
        let source_identity = s.active().unwrap().source.clone();
        assert_eq!(source_identity.as_ref().unwrap().origin, photocraft_io::ImportOrigin::xcf());
        assert!(s.active().unwrap().source_read_only);
        assert!(s.active().unwrap().path.is_none());
        let revision = s.active().unwrap().revision;
        let history_count = s.active().unwrap().history.past_len();
        let prepared = crate::accounted_native_save::PreparedAccountedNativeSave::prepare(&s, "analytic-current.pcraft", &ctl()).unwrap();
        assert_eq!(prepared.snapshot().xcf_state, initial.xcf_state);
        for (a, b) in initial.layers.iter().zip(&prepared.snapshot().layers) {
            match (&a.mask, &b.mask) {
                (Some(a), Some(b)) => assert!(Arc::ptr_eq(&a.surface, &b.surface)),
                (None, None) => {}
                _ => panic!("mask changed during paid titlecopy"),
            }
        }
        let saved = prepared
            .write_and_publish(&mut s, &ctl(), |snapshot, _| {
                photocraft_format::owned_document::save_current_document_archive(snapshot, Cursor::new(Vec::new()), Default::default(), &q, &ctl())
                    .map(|v| v.into_inner())
                    .map_err(|e| EngineError::Other(e.to_string()))
            })
            .unwrap();
        let current = s.active().unwrap();
        assert_eq!(current.doc.name, "analytic-current.pcraft");
        assert_eq!(current.path.as_deref(), Some("analytic-current.pcraft"));
        assert_eq!(current.source, source_identity);
        assert!(current.source_read_only);
        assert_eq!(current.revision, revision);
        assert_eq!(current.history.past_len(), history_count);
        assert!(!current.history.can_undo());
        assert_eq!(native(&s), raw);
        let saved_current = current.doc.clone();
        let mut generic = Session::new();
        assert!(
            generic
                .add_owned_native_archive(archive(&saved, &q), AccountedNativeOpenPolicy::KeepCurrentInterpretation, "must-not-fallback.pcraft", None, &ctl())
                .is_err()
        );
        assert!(generic.documents().is_empty());
        let mut reopened = Session::new();
        reopened
            .add_owned_native_analytic_geometry_archive(archive(&saved, &q), "actual-native.pcraft", Some("authorized/actual-native.pcraft"), &ctl())
            .unwrap();
        let state = reopened.active().unwrap();
        assert_eq!(state.doc.xcf_state, saved_current.xcf_state);
        assert_eq!(state.doc.name, saved_current.name);
        assert_eq!(state.doc.depth, initial.depth);
        assert_eq!(state.doc.mode, initial.mode);
        assert_eq!(state.doc.icc_profile, initial.icc_profile);
        assert_eq!(state.doc.source_color_context, initial.source_color_context);
        assert_eq!(state.doc.working_profile_interpretation, initial.working_profile_interpretation);
        assert_eq!(native(&reopened), raw);
        assert!(state.doc.quota().unwrap().shares_ledger(&q));
        assert_eq!(state.source.as_ref().unwrap().origin, photocraft_io::ImportOrigin::writable(photocraft_io::ImportFormat::Pcraft));
        assert_eq!(state.source.as_ref().unwrap().path.as_deref(), Some("authorized/actual-native.pcraft"));
        assert_eq!(state.path.as_deref(), Some("authorized/actual-native.pcraft"));
        assert!(!state.source_read_only);
        // The saved locator/fingerprint remains inert exclusion evidence; only
        // this authorized native path supplies current writable identity.
        assert_eq!(state.doc.xcf_state.as_ref().unwrap().provenance, initial.xcf_state.as_ref().unwrap().provenance);
        assert_eq!(hash(&bytes), pin);
    }
}
#[test]
#[ignore = "UNRUN: actual analytic geometry source/native publication cancellation/quota/duplicate identity and failed save roll back all state"]
fn analytic_current_publication_and_save_failures_keep_real_owners_and_never_fallback() {
    for (path, pin, len, operator) in CASES {
        let bytes = source(path, pin, len);
        let q = BackingQuota::new(QuotaLimits { resident: 256 << 20, index: 32 << 20, scratch: 0 });
        let owner = imported(path, &bytes, &q);
        let before = q.usage().unwrap();
        let mut s = Session::new();
        let stop = || true;
        let token = Interrupt::cancel_only(&stop);
        let cancel = ReadControl { interrupt: &token, work: &|_| Ok(()) };
        assert!(s.add_owned_analytic_geometry_xcf_import(&owner, path, None, &cancel).is_err());
        assert!(s.documents().is_empty());
        assert_eq!(q.usage().unwrap(), before);
        let fee = q.reserve(QuotaKind::Index, (32 << 20) - q.usage().unwrap().index).unwrap();
        assert!(s.add_owned_analytic_geometry_xcf_import(&owner, path, None, &ctl()).is_err());
        assert!(s.documents().is_empty());
        drop(fee);
        s.add_owned_analytic_geometry_xcf_import(&owner, path, Some(path), &ctl()).unwrap();
        let initial = s.active().unwrap().doc.clone();
        let expected_operator = match operator {
            1 => photocraft_doc::import_compositing::GimpOperator::Union,
            2 => photocraft_doc::import_compositing::GimpOperator::ClipToBackdrop,
            3 => photocraft_doc::import_compositing::GimpOperator::ClipToLayer,
            4 => photocraft_doc::import_compositing::GimpOperator::Intersection,
            _ => panic!("pinned fixture operator"),
        };
        let foreground = initial.layers.last().unwrap();
        let policy = initial
            .xcf_state
            .as_ref()
            .unwrap()
            .items
            .iter()
            .find(|item| item.item == photocraft_doc::item_identity::ItemRef::Layer(foreground.id))
            .unwrap()
            .composition
            .as_ref()
            .unwrap();
        assert_eq!(policy.stored_operator, operator);
        assert_eq!(policy.resolved.as_ref().unwrap().operator, expected_operator);
        let encoding = if path.contains("-linear-evidence/") {
            photocraft_doc::SampleEncoding::Linear
        } else if path.contains("-perceptual-evidence/") {
            photocraft_doc::SampleEncoding::Perceptual
        } else {
            photocraft_doc::SampleEncoding::NonlinearProfile
        };
        assert_eq!(initial.sample_encoding, Some(encoding));
        let raw = native(&s);
        let used = q.usage().unwrap();
        let history = s.active().unwrap().history.clone();
        let revision = s.active().unwrap().revision;
        let source_identity = s.active().unwrap().source.clone();
        let journal = s.journal.clone();
        assert!(s.add_owned_analytic_geometry_xcf_import(&owner, path, None, &ctl()).is_err());
        assert_eq!(s.documents().len(), 1);
        let prepared = crate::accounted_native_save::PreparedAccountedNativeSave::prepare(&s, "failed.pcraft", &ctl()).unwrap();
        assert!(prepared.write_and_publish(&mut s, &ctl(), |_, _| Err::<(), _>(EngineError::Other("intentional writer refusal".into()))).is_err());
        assert!(s.active().unwrap().doc.shares_state(&initial));
        assert!(s.active().unwrap().history.shares_accounted_history(&history));
        assert_eq!(s.active().unwrap().revision, revision);
        assert_eq!(s.active().unwrap().source, source_identity);
        assert!(s.active().unwrap().path.is_none());
        assert_eq!(s.journal, journal);
        assert_eq!(native(&s), raw);
        assert_eq!(q.usage().unwrap(), used);
        let deny = ReadControl { interrupt: &Interrupt::NONE, work: &|_| Err(BackingError::Work) };
        assert_eq!(is_current_analytic_geometry_candidate(&initial, &deny), Err(BackingError::Work));
        assert_eq!(is_current_analytic_geometry_candidate(&initial, &cancel), Err(BackingError::Cancelled));
    }
}
