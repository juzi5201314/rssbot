use coolq_sdk_rust::gen_app_json::AppJson;

fn main() {
    AppJson::new("dev.gugugu.rssbot")
        .name("rssbot".to_owned())
        .author("soeur".to_owned())
        .version("0.0.1".to_owned())
        .version_id(1)
        .finish()
}
