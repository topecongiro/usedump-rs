mod used_item;

use std::{env, io};

fn main() -> io::Result<()> {
    let map = used_item::list_used_items_in_cargo(&env::current_dir()?)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    print!("{}", serde_json::to_string(&map)?);

    Ok(())
}
