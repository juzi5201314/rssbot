#![feature(test)]

extern crate test;

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
struct Entity {
    v: Vec<u8>
}

#[derive(Serialize, Deserialize, Debug)]
struct Value {
    i: i64,
    s: String,
    e: Entity
}

#[bench]
fn reg_clean_html_bench(b: &mut test::Bencher) {
    let reg = Regex::new(r#"<[^>]*>"#).unwrap();
    let html = "<div class=\"push\"><div class=\"body\"><!-- push -->\n<div class=\"d-flex border-bottom py-3\">\n  <span class=\"mr-3\"><a class=\"d-inline-block\" href=\"/juzi5201314\" rel=\"noreferrer\"><img class=\"avatar\" src=\"https://avatars2.githubusercontent.com/u/26034975?s=64&amp;v=4\" width=\"32\" height=\"32\" alt=\"@juzi5201314\"></a></span>\n  <div class=\"d-flex flex-column width-full\">\n    <div>\n      <a class=\"link-gray-dark no-underline text-bold wb-break-all d-inline-block\" href=\"/juzi5201314\" rel=\"noreferrer\">juzi5201314</a>\n      \n      pushed to\n\n        <a class=\"branch-name\" href=\"/juzi5201314/coolq-sdk-rust/tree/master\" rel=\"noreferrer\">master</a>\n        in\n\n      <a class=\"link-gray-dark no-underline text-bold wb-break-all d-inline-block\" href=\"/juzi5201314/coolq-sdk-rust\" rel=\"noreferrer\">juzi5201314/coolq-sdk-rust</a>\n      <span class=\"f6 text-gray-light no-wrap ml-1\">\n        <relative-time datetime=\"2020-02-25T08:24:22Z\" class=\"no-wrap\">Feb 25, 2020</relative-time>\n      </span>\n\n        <div class=\"Box p-3 mt-2\">\n          <span>2 commits to</span>\n          <a class=\"branch-name\" href=\"/juzi5201314/coolq-sdk-rust/tree/master\" rel=\"noreferrer\">master</a>\n\n          <div class=\"commits pusher-is-only-committer\">\n            <ul>\n                <li class=\"d-flex flex-items-baseline\">\n                  <span title=\"juzi5201314\">\n                    <a class=\"d-inline-block\" href=\"/juzi5201314\" rel=\"noreferrer\"><img class=\"mr-1\" src=\"https://avatars1.githubusercontent.com/u/26034975?s=32&amp;v=4\" width=\"16\" height=\"16\" alt=\"@juzi5201314\"></a>\n                  </span>\n                  <code><a class=\"mr-1\" href=\"/juzi5201314/coolq-sdk-rust/commit/b3619a50b2f1f6a9dcc2555290a659b8c43aef63\" rel=\"noreferrer\">b3619a5</a></code>\n                  <div class=\"dashboard-break-word lh-condensed\">\n                    <blockquote>\n                      Merge remote-tracking branch \'origin/master\'\n                    </blockquote>\n                  </div>\n                </li>\n                <li class=\"d-flex flex-items-baseline\">\n                  <span title=\"juzi5201314\">\n                    <a class=\"d-inline-block\" href=\"/juzi5201314\" rel=\"noreferrer\"><img class=\"mr-1\" src=\"https://avatars1.githubusercontent.com/u/26034975?s=32&amp;v=4\" width=\"16\" height=\"16\" alt=\"@juzi5201314\"></a>\n                  </span>\n                  <code><a class=\"mr-1\" href=\"/juzi5201314/coolq-sdk-rust/commit/b6f306aa245017d75a44a4f8ca5f6c6f98b3a302\" rel=\"noreferrer\">b6f306a</a></code>\n                  <div class=\"dashboard-break-word lh-condensed\">\n                    <blockquote>\n                      现在new一个group和user的时候，如果酷q api失败不会panic了，会返回一个只有id，其他消息都是default的struct。\n                    </blockquote>\n                  </div>\n                </li>\n\n\n                <li class=\"f6 mt-2\">\n                  <a class=\"link-gray\" href=\"/juzi5201314/coolq-sdk-rust/compare/af947443b9...b3619a50b2\" rel=\"noreferrer\">3 more commits »</a>\n                </li>\n            </ul>\n          </div>\n        </div>\n    </div>\n  </div>\n</div>\n</div></div>";
    b.iter(|| {
        reg.replace_all(test::black_box(html), "").replace("\n", "").replace(" ", "")
    });
}

#[bench]
fn big_hash(b: &mut test::Bencher) {
    let str = "a".repeat(1024);
    b.iter(|| {
        let mut hasher = DefaultHasher::new();
        str.hash( test::black_box(&mut hasher));
        hasher.finish();
    })
}

#[bench]
fn hash_link(b: &mut test::Bencher) {
    let link = test::black_box("https://google.com");
    b.iter(|| {
        let mut hasher = DefaultHasher::new();
        link.hash( test::black_box(&mut hasher));
        hasher.finish();
    })
}

#[bench]
fn decode_bincode(b: &mut test::Bencher) {
    let v = Value {
        i: 10086,
        s: "this ok".to_string(),
        e: Entity {
            v: vec![1, 4, 5]
        }
    };
    b.iter(|| {
        let _ = bincode::serialize(test::black_box(&v)).unwrap();
    })
}

#[bench]
fn decode_json(b: &mut test::Bencher) {
    let v = Value {
        i: 10086,
        s: "this ok".to_string(),
        e: Entity {
            v: vec![1, 4, 5]
        }
    };
    b.iter(|| {
        let _ = serde_json::to_string(test::black_box(&v)).unwrap();
    })
}

#[bench]
fn decode_cbor(b: &mut test::Bencher) {
    let v = Value {
        i: 10086,
        s: "this ok".to_string(),
        e: Entity {
            v: vec![1, 4, 5]
        }
    };
    b.iter(|| {
        let _ = serde_cbor::to_vec(test::black_box(&v)).unwrap();
    })
}

#[bench]
fn decode_msgpack(b: &mut test::Bencher) {
    let v = Value {
        i: 10086,
        s: "this ok".to_string(),
        e: Entity {
            v: vec![1, 4, 5]
        }
    };
    b.iter(|| {
        let mut vec = Vec::new();
        let mut serializer = rmp_serde::Serializer::new(&mut vec);
        v.serialize(&mut serializer).unwrap();
    })
}

#[bench]
fn decode_postcard(b: &mut test::Bencher) {
    let v = Value {
        i: 10086,
        s: "this ok".to_string(),
        e: Entity {
            v: vec![1, 4, 5]
        }
    };
    b.iter(|| {
        let _: heapless::Vec<u8, heapless::consts::U100000> = postcard::to_vec(test::black_box(&v)).unwrap();
    })
}