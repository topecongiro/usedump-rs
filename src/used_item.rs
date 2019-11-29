use std::collections::{BTreeMap, BTreeSet};

use ra_db::{FileId, SourceDatabaseExt};
use ra_ide::{Analysis, FilePosition, NavigationTarget};
use ra_syntax::{
    ast::{ModuleItem, ModuleItemOwner, UseItem, UseTree},
    AstNode, SyntaxKind,
};
use serde::{Serialize, Serializer};

pub fn list_used_items_in_cargo<Q: AsRef<std::path::Path>>(
    dir: Q,
) -> Result<CrateMap, Box<dyn std::error::Error + Send + Sync>> {
    let (analysis_host, source_map) = ra_batch::load_cargo(dir.as_ref())?;

    let analysis = analysis_host.analysis();
    let mut map = CrateMap::default();

    for (source_root_id, package_root) in source_map {
        if !package_root.is_member() {
            continue;
        }

        for file_id in analysis_host
            .raw_database()
            .source_root(source_root_id)
            .walk()
        {
            let resolver = UsedItemResolver::new(&analysis, file_id);
            let path = analysis_host.raw_database().file_relative_path(file_id);
            map.source_map
                .insert(path.to_string(), resolver.used_items());
        }
    }

    Ok(map)
}

#[derive(Default, Serialize)]
pub struct CrateMap {
    #[serde(flatten)]
    source_map: BTreeMap<String, UsedItemMap>,
}

#[derive(Debug, PartialOrd, PartialEq, Eq, Ord)]
pub enum UsedItemKind {
    Module,
    Trait,
    Struct,
    Enum,
    Fn,
    Const,
    Macro,
    Other,
}

impl UsedItemKind {
    fn from_syntax_kind(syntax_kind: SyntaxKind) -> Self {
        match syntax_kind {
            SyntaxKind::SOURCE_FILE => UsedItemKind::Module,
            SyntaxKind::TRAIT_DEF => UsedItemKind::Trait,
            SyntaxKind::STRUCT_DEF => UsedItemKind::Struct,
            SyntaxKind::ENUM_DEF => UsedItemKind::Enum,
            SyntaxKind::FN_DEF => UsedItemKind::Fn,
            SyntaxKind::CONST_DEF => UsedItemKind::Const,
            SyntaxKind::MACRO_CALL => UsedItemKind::Macro,
            _ => UsedItemKind::Other,
        }
    }
}

#[derive(Debug, PartialOrd, PartialEq, Eq, Ord)]
pub struct UsedItem {
    name: String,
    kind: UsedItemKind,
}

impl Serialize for UsedItem {
    fn serialize<S>(&self, serializer: S) -> Result<<S as Serializer>::Ok, <S as Serializer>::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.name)
    }
}

impl UsedItem {
    fn from_navigation_target(navigation_target: &NavigationTarget) -> Self {
        let name = navigation_target.name().to_string();
        let kind = UsedItemKind::from_syntax_kind(navigation_target.kind());
        UsedItem { name, kind }
    }
}

struct UsedItemResolver<'a> {
    analysis: &'a Analysis,
    file_id: FileId,
    used_item_map: UsedItemMap,
}

#[derive(Default, Debug, Serialize)]
pub struct UsedItemMap {
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    modules: BTreeSet<UsedItem>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    traits: BTreeSet<UsedItem>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    structs: BTreeSet<UsedItem>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    enums: BTreeSet<UsedItem>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    fns: BTreeSet<UsedItem>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    consts: BTreeSet<UsedItem>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    macros: BTreeSet<UsedItem>,
    #[serde(skip_serializing_if = "BTreeSet::is_empty")]
    others: BTreeSet<UsedItem>,
}

impl<'a> UsedItemResolver<'a> {
    fn new(analysis: &'a Analysis, file_id: FileId) -> Self {
        UsedItemResolver {
            file_id,
            analysis,
            used_item_map: Default::default(),
        }
    }

    fn used_items(mut self) -> UsedItemMap {
        let source_file = match self.analysis.parse(self.file_id) {
            Ok(s) => s,
            Err(_) => return UsedItemMap::default(),
        };

        for use_item in source_file.items().filter_map(item_to_use_item) {
            if let Some(imported_items) = self.used_items_in_use_item(&use_item) {
                self.add_imported_items(imported_items);
            }
        }

        self.used_item_map
    }

    fn add_imported_items(&mut self, used_items: Vec<UsedItem>) {
        for item in used_items {
            use UsedItemKind::*;
            match item.kind {
                Module => self.used_item_map.modules.insert(item),
                Trait => self.used_item_map.traits.insert(item),
                Struct => self.used_item_map.structs.insert(item),
                Enum => self.used_item_map.enums.insert(item),
                Fn => self.used_item_map.fns.insert(item),
                Const => self.used_item_map.consts.insert(item),
                Macro => self.used_item_map.macros.insert(item),
                Other => self.used_item_map.others.insert(item),
            };
        }
    }

    fn used_items_in_use_item(&self, use_item: &UseItem) -> Option<Vec<UsedItem>> {
        self.used_items_in_use_tree(&use_item.use_tree()?)
    }

    fn used_items_in_use_tree(&self, use_tree: &UseTree) -> Option<Vec<UsedItem>> {
        match use_tree.use_tree_list() {
            Some(use_tree_list) => {
                let mut result = vec![];
                for use_tree in use_tree_list.use_trees() {
                    if let Some(mut items) = self.used_items_in_use_tree(&use_tree) {
                        result.append(&mut items);
                    }
                }

                Some(result)
            }
            None => {
                let offset = use_tree.syntax().text_range().end();
                let file_position = FilePosition {
                    file_id: self.file_id,
                    offset,
                };
                if let Ok(Some(range_info)) = self.analysis.goto_definition(file_position) {
                    Some(
                        range_info
                            .info
                            .iter()
                            .map(UsedItem::from_navigation_target)
                            .collect(),
                    )
                } else {
                    None
                }
            }
        }
    }
}

fn item_to_use_item(item: ModuleItem) -> Option<UseItem> {
    match item {
        ModuleItem::UseItem(use_item) => Some(use_item),
        _ => None,
    }
}
