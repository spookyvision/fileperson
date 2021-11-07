use directories::UserDirs;
use fileperson::State;

fn main() {
    pretty_env_logger::init();
    let root = UserDirs::new().unwrap();
    let root = root.desktop_dir().unwrap();

    State::files("/Users/anatol.ulrich/Desktop");
    println!("Hello, world!");
}
