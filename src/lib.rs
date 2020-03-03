#![allow(unused_must_use)]

use std::cmp::min;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicI64, Ordering};

use atom_syndication::Link;
use chrono::{TimeZone, Local};
use coolq_sdk_rust::api::{add_log, get_app_directory, get_group_list, get_login_qq, send_group_msg};
use coolq_sdk_rust::prelude::*;
use coolq_sdk_rust::targets::user::Authority;
use once_cell::sync::Lazy;
use regex::Regex;
use rss::{Channel, Item};
use rss::validation::Validate;
use serde::{Deserialize, Serialize};
use sled::{IVec, Tree};
use tokio::runtime::Runtime;
use tokio::sync::RwLock;
use tokio::time::{Duration, interval};

static DATABASE: Lazy<RwLock<sled::Db>> = Lazy::new(|| {
    RwLock::new(
        sled::open(
            get_app_directory()
                .expect("无法获取应用目录")
                .to::<String>(),
        )
            .expect("无法打开数据库"),
    )
});

static RUNTIME: Lazy<Runtime> = Lazy::new(|| Runtime::new().unwrap());

static REG: Lazy<Regex> = Lazy::new(|| Regex::new(r#"<[^>]*>"#).unwrap());

static ME_QQ: AtomicI64 = AtomicI64::new(0);

#[coolq_sdk_rust::main]
fn main() {
    // 本人qq
    User::add_master(1034236490);
    ME_QQ.store(get_login_qq().unwrap().into(), Ordering::Relaxed);

    RUNTIME.spawn(async {
        let mut interval = interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            update_all_rss(false).await;
        }
    });
}

#[listener(event = "AddGroupRequestEvent")]
fn join_group(event: &mut AddGroupRequestEvent) {
    // 受到管理员邀请自动同意入群邀请
    if event.is_invite() && event.user.authority.check_authority(Authority::SuperAdmin) {
        event.handle(true, "");
    }
}

#[listener(event = "GroupMessageEvent")]
fn gm(event: &mut GroupMessageEvent) {
    // 如果有权限并且在at机器人
    if event.user.authority.check_authority(Authority::GroupAdmin) /*&& !event.get_message().cqcodes.iter().all(|code| {
        if let &CQCode::At(qq) = code {
            ME_QQ.load(Ordering::Relaxed) != qq
        } else {
            true
        }
    })*/ {
        let msg = &event.get_message().msg.trim();
        if let Some(s) = msg.get(0..1) {
            if s == "/" && msg.len() > 1 {
                let args = (&msg[1..]).split(' ').map(|s| s.to_owned()).collect::<Vec<String>>();
                let event = event.clone();
                RUNTIME.spawn(async move {
                    if let Err(err) = process_command(&event, args).await {
                        event.reply(MessageSegment::new()
                            .add("Error: ")
                            .add(err.0)
                        );
                    }
                });
            }
        }
    }
}

struct CommandError(String);

impl From<String> for CommandError {
    fn from(s: String) -> Self {
        CommandError(s)
    }
}

impl From<sled::Error> for CommandError {
    fn from(err: sled::Error) -> Self {
        err.to_string().into()
    }
}

macro_rules! check_args {
    ($args: expr, $count: expr, $help: expr) => {
        if $args.len() < $count {
            return Err(CommandError::from(String::from($help)))
        }
    };
}

// 将错误转换为字符串
macro_rules! check_err {
    ($result: expr, $err_msg: expr, $raw_err: expr) => {
        match $result {
            Ok(ok) => ok,
            Err(err) => return Err(format!("{}\n{}", $err_msg, if $raw_err { err.to_string() } else { "-- What's your problem?".to_owned() }).into())
        }
    };

    ($result: expr, $err_msg: expr) => {
        check_err!($result, $err_msg, true)
    };

    ($result: expr) => {
        check_err!($result, "", true)
    };
}

#[derive(Serialize, Deserialize, Debug, Default)]
struct RssValue {
    title: String,
    groups: Vec<i64>,
    last_update: i64,
    item_uuid: Vec<u64>,
    update_interval: i64,
}

impl RssValue {
    fn serialize(&self) -> bincode::Result<Vec<u8>> {
        bincode::serialize(self)
    }

    fn deserialize(b: &[u8]) -> bincode::Result<RssValue> {
        bincode::deserialize(b)
    }
}

#[inline]
async fn open_rsshub() -> sled::Result<Tree> {
    DATABASE.read().await.open_tree("rsshub")
}

// 判断此群是否存在此rss订阅
// 如果不存在rss链接，返回空RssValue
// 如果存在rss链接但此群不存在该rss订阅，返回该rss订阅的RssValue
// 如果如果存在rss链接且该群存在rss订阅，返回None
fn contains_and_get_rss(tree: &Tree, group: i64, link: &str) -> Result<Option<RssValue>, CommandError> {
    let key = link.as_bytes();
    if check_err!(tree.contains_key(key)) {
        let v = check_err!(tree.get(key)).unwrap();
        let v = check_err!(RssValue::deserialize(v.as_ref()), "rss decode失败", true);
        Ok(if v.groups.iter().find(|group_id| **group_id == group).is_some() { None } else { Some(v) })
    } else {
        Ok(Some(RssValue::default()))
    }
}

fn hash(s: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    REG.replace_all(s, "").replace("\n", "").replace(" ", "").hash(&mut hasher);
    hasher.finish()
}

async fn process_command(event: &GroupMessageEvent, args: Vec<String>) -> Result<(), CommandError> {
    if args.get(0).unwrap() == &"rss" {
        match args.get(1).unwrap_or(&String::from("help")).as_str() {
            "help" => {
                event.reply("\
                --help--\n\
                /rss add <url> [no_validate]\n\
                /rss del <url>\n\
                /rss list\n\
                /rss status\
                ");
            }
            "list" => {
                let tree = check_err!(open_rsshub().await, "数据库打开失败", true);
                let rss = tree.iter().filter_map(|kv| {
                    let kv = kv.as_ref().unwrap();
                    let value = match RssValue::deserialize(kv.1.as_ref()) {
                        Ok(value) => value,
                        Err(err) => return Some(err.to_string())
                    };
                    if value.groups.iter().find(|group_id| **group_id == event.group.group_id).is_some() {
                        Some(format!("{}({}). LU: {}. TTL: {}s. NEXT: {}s.",
                                     value.title,
                                     String::from_utf8(kv.0.to_vec()).unwrap(),
                                     chrono::Local.timestamp(value.last_update, 0).to_string(),
                                     value.update_interval,
                                     (value.last_update + value.update_interval) - chrono::Local::now().timestamp()
                        ))
                    } else {
                        None
                    }
                }).collect::<Vec<String>>();
                if rss.is_empty() {
                    event.reply("没有rss订阅");
                } else {
                    // 分页，每页5个
                    let page_count = 5;
                    let mut page_i = 0;
                    for _ in 0..rss.len() / page_count {
                        let page = &rss[page_i..page_i + page_count];
                        page_i += page_count;
                        event.reply(page.join("\n\n"));
                    }
                    if rss.len() % page_count > 0 {
                        event.reply(&rss[(rss.len() / page_count) * page_count..].join("\n\n"));
                    }
                }
            }
            "add" => {
                check_args!(args, 3, "/rss add <url> [no_validate]");
                //let url = check_err!(Url::parse(args.get(2).unwrap()), "请输入正确url", true);
                let url = args.get(2).unwrap();
                let url = url.as_str();
                let tree = check_err!(open_rsshub().await, "数据库打开失败", true);
                let rss = contains_and_get_rss(&tree, event.group.group_id, url)?;
                if rss.is_none() {
                    return Err("此rss已存在".to_owned().into());
                }
                let mut v = rss.unwrap();
                v.groups.push(event.group.group_id);
                let channel = get_channel(url).await?;
                if args.get(3).unwrap_or(&String::new()) != "no_validate" {
                    check_err!(channel.validate(), "rss不合法");
                }
                if !channel.items().iter().all(|item| item.link().is_some()) {
                    return Err("rss不合法".to_owned().into());
                }
                //新的RssValue
                if v.last_update <= 0 {
                    v.last_update = chrono::Local::now().timestamp();
                    v.title = channel.title().to_string();
                    v.item_uuid = channel.items().iter().map(|item| {
                        hash(item.pub_date().unwrap_or(item.link().unwrap().trim()))
                    }).collect();
                    // 抓取间隔。单位： 分钟
                    // 如未设置ttl，默认10分钟。最大90分钟。
                    v.update_interval = min(channel.ttl().unwrap_or_default().parse::<i64>().unwrap_or(10), 90) * 60;
                }
                check_err!(tree.insert(url.as_bytes(), check_err!(v.serialize(), "RssValue序列化失败", true)), "插入数据库失败");
                event.reply(MessageSegment::new()
                    .add("完成!")
                    .newline()
                    .add(channel.title())
                    .newline()
                    .add("ttl: ")
                    .add(v.update_interval));
            }
            "del" => {
                check_args!(args, 3, "/rss del <url>");
                let tree = check_err!(open_rsshub().await, "数据库打开失败", true);
                let url = args.get(2).unwrap();
                let rss = contains_and_get_rss(&tree, event.group.group_id, url)?;
                if rss.is_none() {
                    tree.remove(url.as_bytes());
                    event.reply(MessageSegment::new().add("已删除: ").add(url));
                } else {
                    return Err("此rss订阅不存在".to_owned().into());
                }
            },
            "ttl" => {
                if event.user.authority.check_authority(Authority::SuperAdmin) {
                    check_args!(args, 4, "/rss ttl <url> <secs>");
                    let tree = check_err!(open_rsshub().await, "数据库打开失败", true);
                    let url = args.get(2).unwrap();
                    let rss = contains_and_get_rss(&tree, event.group.group_id, url)?;
                    if rss.is_none() {
                        let mut v = RssValue::deserialize(tree.get(url.as_bytes()).unwrap().unwrap().as_ref()).unwrap();
                        if let Ok(ui) = args.get(3).unwrap().parse::<i64>() {
                            v.update_interval = ui;
                            tree.insert(url.as_bytes(), v.serialize().unwrap());
                            event.reply(format!("({})更新间隔为: {}s", url, ui));
                        }
                    } else {
                        return Err("此rss订阅不存在".to_owned().into());
                    }
                }
            },
            "update" => {
                if event.user.authority.check_authority(Authority::SuperAdmin) {
                    update_all_rss(true).await;
                }
            },
            "status" => {
                let db = DATABASE.read().await;
                let tree = check_err!(db.open_tree("rsshub"));
                event.reply(format!("\
                数据库大小: {}KB\n\
                rss源: {}条\
                ", db.size_on_disk().unwrap() / 1024,
                tree.len()));
            },
            "clear" => {
                if event.user.authority.check_authority(Authority::SuperAdmin) {
                    let tree = check_err!(open_rsshub().await, "数据库打开失败", true);
                    let groups: Vec<i64> = check_err!(check_err!(get_group_list()).try_to::<Vec<Group>>()).iter().map(|g| g.group_id).collect();
                    let mut dead = Vec::new();
                    tree.iter().for_each(|kv| {
                        let kv = kv.unwrap();
                        let mut v = RssValue::deserialize(kv.1.as_ref()).unwrap();
                        v.groups = v.groups.iter().filter_map(|gid| if groups.contains(gid) { Some(*gid) } else {
                            dead.push(*gid);
                            None
                        }).collect();
                        tree.insert(kv.0, v.serialize().unwrap());
                    });
                    event.reply(format!("共清理{}个群。\n{:?}", dead.len(), dead));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

#[inline]
fn truncate(s: &str, max_chars: usize) -> String {
    match s.char_indices().nth(max_chars) {
        None => s.to_owned(),
        Some((idx, _)) => format!("{} ...", &s[..idx]),
    }
}

#[inline]
fn html_to_text(html: &str, width: usize) -> String {
    html2text::from_read_with_decorator(html.as_bytes(), width, html2text::render::text_renderer::TrivialDecorator::new())
}

fn atom_to_rss(bytes: &[u8]) -> Result<Channel, String> {
    let feed = check_err!(atom_syndication::Feed::read_from(bytes));
    let mut channel = Channel::default();
    channel.set_title(feed.title());
    channel.set_link(feed.links().get(0).unwrap_or(&Link::default()).href());
    channel.set_items(feed.entries().iter().map(|e| {
        let mut item = Item::default();
        item.set_title(e.title().to_owned());
        item.set_link(if let Some(link) = e.links().get(0) {
            Some(link.href().to_owned())
        } else {
            None
        });

        item.set_content(if let Some(content) = e.content() {
            Some(content.value().unwrap_or_default().to_owned())
        } else {
            None
        });
        item.set_pub_date(e.updated().to_rfc2822());
        item
    }).collect::<Vec<Item>>());
    Ok(channel)
}

async fn get_channel(url: &str) -> Result<Channel, String> {
    let client = reqwest::ClientBuilder::new()
        .timeout(Duration::from_secs(10))
        .no_proxy()
        .build()
        .unwrap();
    let rss_content = check_err!(client.get(url).send().await, "获取rss信息失败", true);
    let bytes = check_err!(rss_content.text().await, "读取rss内容失败");
    let bytes = bytes.as_bytes();
    let rss = rss::Channel::read_from(bytes);
    match rss {
        Ok(channel) => Ok(channel),
        Err(err) => {
            if let rss::Error::InvalidStartTag = err {
                atom_to_rss(bytes)
            } else {
                Err(err.to_string())
            }
        }
    }
}

async fn update_all_rss(force: bool) {
    let tree = open_rsshub().await.unwrap();
    let v = tree.iter().collect::<Vec<sled::Result<(IVec, IVec)>>>();
    for kv in v {
        let (key, value) = kv.unwrap();
        let (rssurl, mut rssvalue) = (String::from_utf8(key.to_vec()).unwrap(), RssValue::deserialize(value.as_ref()).unwrap());
        if !force && chrono::Local::now().timestamp() - rssvalue.last_update < rssvalue.update_interval {
            continue;
        }
        let channel = match get_channel(&rssurl).await {
            Ok(channel) => channel,
            Err(err) => {
                add_log(CQLogLevel::WARNING, "update_all_rss", format!("抓取失败: {}\nError: {}", &rssurl, err));
                continue;
            }
        };
        let mut new_items = Vec::with_capacity(channel.items().len());
        let limit = 5; //限制一次才能发5条消息，防止突然刷屏
        let mut now = 0;
        channel.items().iter().for_each(|item| {
            if now >= limit {
                return;
            }
            now += 1;
            let id = hash(item.pub_date().unwrap_or(item.link().unwrap().trim()));
            new_items.push(id);
            if !rssvalue.item_uuid.contains(&id) {
                let time = if let Some(t) = item.pub_date() {
                    if let Ok(t) = chrono::DateTime::parse_from_rfc2822(t) {
                        //t.checked_add_signed(time::Duration::seconds(t.offset().local_minus_utc() as i64)).unwrap().format("%F %T").to_string()
                        t.with_timezone(&Local).format("%F %T").to_string()
                    } else {
                        String::from("非法时间格式")
                    }
                } else {
                    String::from("未知时间")
                };
                // update
                rssvalue.groups.iter().for_each(|group_id| {
                    fn send(group_id: i64, ms: &MessageSegment) {
                        if let Err(e) = send_group_msg(group_id, ms) {
                            if e.0 == -2 {
                                add_log(CQLogLevel::WARNING, "消息发送失败", format!("重新发送: {}", ms));
                                send(group_id, ms)
                            }
                        }
                    }
                    send(*group_id, MessageSegment::new()
                        .add(channel.title())
                        .newlines(2)
                        .add(format!("{} --{}", item.title().unwrap_or_default().replace("\n", "").trim(), &time))
                        .newlines(2)
                        .add(item.link().unwrap_or_default().trim())
                        .newlines(2)
                        .add(truncate(html_to_text(item.description().unwrap_or(item.content().unwrap_or_default()), 28).as_ref(), 100))
                    );
                    //add_log(10, "迁移中", format!("迁移: {}", rssurl));
                })
            }
        });
        rssvalue.last_update = chrono::Local::now().timestamp();
        rssvalue.item_uuid = new_items;
        tree.insert(key, rssvalue.serialize().unwrap());
        add_log(CQLogLevel::DEBUG, "update_all_rss", format!("更新订阅: {}({})", channel.title(), rssurl));
    }
}

#[test]
fn test_clean_html() {
    let r = Regex::new(r#"\n\s*\n"#).unwrap();
    let reg = Regex::new(r#"<[^>]*>"#).unwrap();
    let html = "\n<div class=\"push\"><p>ennc</p><div class=\"body\"><!-- push -->\n<div class=\"d-flex border-bottom py-3\">\n  <span class=\"mr-3\"><a class=\"d-inline-block\" href=\"/juzi5201314\" rel=\"noreferrer\"><img class=\"avatar\" src=\"https://avatars2.githubusercontent.com/u/26034975?s=64&amp;v=4\" width=\"32\" height=\"32\" alt=\"@juzi5201314\"></a></span>\n  <div class=\"d-flex flex-column width-full\">\n    <div>\n      <a class=\"link-gray-dark no-underline text-bold wb-break-all d-inline-block\" href=\"/juzi5201314\" rel=\"noreferrer\">juzi5201314</a>\n      \n      pushed to\n\n        <a class=\"branch-name\" href=\"/juzi5201314/coolq-sdk-rust/tree/master\" rel=\"noreferrer\">master</a>\n        in\n\n      <a class=\"link-gray-dark no-underline text-bold wb-break-all d-inline-block\" href=\"/juzi5201314/coolq-sdk-rust\" rel=\"noreferrer\">juzi5201314/coolq-sdk-rust</a>\n      <span class=\"f6 text-gray-light no-wrap ml-1\">\n        <relative-time datetime=\"2020-02-25T08:24:22Z\" class=\"no-wrap\">Feb 25, 2020</relative-time>\n      </span>\n\n        <div class=\"Box p-3 mt-2\">\n          <span>2 commits to</span>\n          <a class=\"branch-name\" href=\"/juzi5201314/coolq-sdk-rust/tree/master\" rel=\"noreferrer\">master</a>\n\n          <div class=\"commits pusher-is-only-committer\">\n            <ul>\n                <li class=\"d-flex flex-items-baseline\">\n                  <span title=\"juzi5201314\">\n                    <a class=\"d-inline-block\" href=\"/juzi5201314\" rel=\"noreferrer\"><img class=\"mr-1\" src=\"https://avatars1.githubusercontent.com/u/26034975?s=32&amp;v=4\" width=\"16\" height=\"16\" alt=\"@juzi5201314\"></a>\n                  </span>\n                  <code><a class=\"mr-1\" href=\"/juzi5201314/coolq-sdk-rust/commit/b3619a50b2f1f6a9dcc2555290a659b8c43aef63\" rel=\"noreferrer\">b3619a5</a></code>\n                  <div class=\"dashboard-break-word lh-condensed\">\n                    <blockquote>\n                      Merge remote-tracking branch \'origin/master\'\n                    </blockquote>\n                  </div>\n                </li>\n                <li class=\"d-flex flex-items-baseline\">\n                  <span title=\"juzi5201314\">\n                    <a class=\"d-inline-block\" href=\"/juzi5201314\" rel=\"noreferrer\"><img class=\"mr-1\" src=\"https://avatars1.githubusercontent.com/u/26034975?s=32&amp;v=4\" width=\"16\" height=\"16\" alt=\"@juzi5201314\"></a>\n                  </span>\n                  <code><a class=\"mr-1\" href=\"/juzi5201314/coolq-sdk-rust/commit/b6f306aa245017d75a44a4f8ca5f6c6f98b3a302\" rel=\"noreferrer\">b6f306a</a></code>\n                  <div class=\"dashboard-break-word lh-condensed\">\n                    <blockquote>\n                      现在new一个group和user的时候，如果酷q api失败不会panic了，会返回一个只有id，其他消息都是default的struct。\n                    </blockquote>\n                  </div>\n                </li>\n\n\n                <li class=\"f6 mt-2\">\n                  <a class=\"link-gray\" href=\"/juzi5201314/coolq-sdk-rust/compare/af947443b9...b3619a50b2\" rel=\"noreferrer\">3 more commits »</a>\n                </li>\n            </ul>\n          </div>\n        </div>\n    </div>\n  </div>\n</div>\n</div></div>";
    //dbg!(r.replace_all(&reg.replace_all(html, "").to_string(), "两个").replace(" ", ""));
    println!("{:?}", html2text::from_read_with_decorator(html.as_bytes(), 15, html2text::render::text_renderer::TrivialDecorator::new()));
}

#[test]
fn bincode_test() {
    #[derive(Serialize, Deserialize, Debug, Default)]
    struct A {
        a: i32
    }
    #[derive(Serialize, Deserialize, Debug, Default)]
    struct B {
        bdwadawdwaf: i32
    }
    assert_eq!(bincode::serialize(&A::default()).unwrap(), bincode::serialize(&B::default()).unwrap());
}


#[test]
fn rss_test() {
    Runtime::new().unwrap().block_on(async {
        let rssurl = "https://rust.cc/rss";
        let channel = get_channel(rssurl).await.unwrap();
        assert_eq!(channel.link(), "https://rust.cc");
        //dbg!(channel);
    });
}

#[test]
fn sled_test() {
    let db = sled::Config::new().temporary(true).open().unwrap();
    let sub = db.open_tree("sub").unwrap();

    sub.insert("a", "1").unwrap();
    sub.insert("a", "12").unwrap();

    assert_eq!(sub.get("a").unwrap().unwrap(), b"12");
    //dbg!(db.tree_names().iter().map(|name| String::from_utf8(name.as_ref().to_vec()).unwrap()).collect::<Vec<String>>());
}

/*
#[test]
fn hotpot() {
    use hotpot_db::*;
use serde::{Deserialize, Serialize};

#[derive(Deserialize, Serialize, Debug)]
struct Rss {
    name: String,
    url: String,
    update_time: usize
}

let mut pot = HotPot::new(".");

pot.create_collection("rsshub").unwrap();

*//*pot.insert::<Rss>("rsshub", &Rss {
    name: "rust.cc".to_string(),
    url: "https://rust.cc/rss".to_string(),
    update_time: 60
});*//*

let query = QueryBuilder::new()
.collection("rsshub")
.kind(QueryKind::Object)
.key("name")
.comparison("=")
.string("rust.cc")
.finish();

let result = pot.execute(query);
dbg!(&result.unwrap());
}*/
