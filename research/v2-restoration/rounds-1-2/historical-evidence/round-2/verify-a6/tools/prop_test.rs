
// ---- verifier's differential test (appended to statics.rs in a scratch copy only) ----
#[cfg(test)]
mod verifier_tests {
    use super::*;
    use crate::item::Linkage;

    fn item(name: &str, kind: ItemKind, refs: &[String]) -> LinkItem {
        LinkItem {
            name: JsName::new(name.to_owned()),
            kind,
            js: format!("let {name} = [{}];\n", refs.join(", ")),
            refs: refs.iter().map(|r| JsName::new(r.clone())).collect(),
            zombies: Vec::new(),
            linkage: Linkage::internal(),
            debug_path: format!("path::{name}"),
            crate_name: "test".to_owned(),
            order: None,
            map: ItemMap::default(),
            dts: None,
            optimization: Default::default(),
            late_ir: Default::default(),
            baseline_shadow: Default::default(),
            jir: Default::default(),
        }
    }

    /// `crate::link::order` as it is on main (ea5604f), copied verbatim.
    fn old_order<'a>(
        table: &'a BTreeMap<JsName, LinkItem>,
        reachable: &'a [JsName],
    ) -> Vec<&'a LinkItem> {
        let mut names: Vec<&JsName> = reachable.iter().collect();
        names.sort();
        names.sort_by_key(|name| {
            let order = table.get(*name).and_then(|item| item.order.clone());
            (order.is_none(), order)
        });
        let mut out: Vec<&LinkItem> = Vec::with_capacity(names.len());
        let mut placed: HashSet<&JsName> = HashSet::new();
        let mut imports: Vec<&LinkItem> = names
            .iter()
            .filter_map(|name| table.get(*name))
            .filter(|item| item.kind == ItemKind::Import)
            .collect();
        imports.sort_by(|a, b| (&a.debug_path, &a.name).cmp(&(&b.debug_path, &b.name)));
        out.extend(imports);
        fn visit<'a>(
            name: &'a JsName,
            table: &'a BTreeMap<JsName, LinkItem>,
            placed: &mut HashSet<&'a JsName>,
            out: &mut Vec<&'a LinkItem>,
        ) {
            let Some((key, item)) = table.get_key_value(name) else {
                return;
            };
            if !item.kind.is_binding() || !placed.insert(key) {
                return;
            }
            for referenced in &item.refs {
                if table
                    .get(referenced)
                    .is_some_and(|item| item.kind.is_binding())
                {
                    visit(referenced, table, placed, out);
                }
            }
            out.push(item);
        }
        for name in &names {
            visit(name, table, &mut placed, &mut out);
        }
        for name in &names {
            if let Some(item) = table.get(*name) {
                if !item.kind.is_binding() && item.kind != ItemKind::Import {
                    out.push(item);
                }
            }
        }
        out
    }

    struct Rng(u64);
    impl Rng {
        fn next(&mut self) -> usize {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 33) as usize
        }
    }

    fn kind_of(r: usize) -> ItemKind {
        match r % 20 {
            0..=11 => ItemKind::Static,
            12 | 13 => ItemKind::Vtable,
            14 | 15 => ItemKind::Const,
            16..=18 => ItemKind::Fn,
            _ => ItemKind::Import,
        }
    }

    /// Everything reachable from `roots` over every reference, as the link step's `reach` does.
    fn closure(table: &BTreeMap<JsName, LinkItem>, roots: &[JsName]) -> Vec<JsName> {
        let mut seen: Vec<JsName> = Vec::new();
        let mut work: Vec<JsName> = roots.to_vec();
        while let Some(name) = work.pop() {
            if seen.contains(&name) {
                continue;
            }
            if let Some(item) = table.get(&name) {
                work.extend(item.refs.iter().cloned());
                seen.push(name);
            }
        }
        seen
    }

    /// On graphs without a cycle among bindings, the new `order` is main's `order`, item for item:
    /// random DAGs of every item kind, names in an order unrelated to the dependency order,
    /// duplicated and shuffled references, some items carrying a source order, and reachable sets
    /// grown from random roots as well as the whole table.
    #[test]
    fn acyclic_graphs_keep_mains_order() {
        let mut rng = Rng(0x1234_5678_9abc_def0);
        let mut compared = 0;
        for _ in 0..20000 {
            let n = 1 + rng.next() % 16;
            let mut perm: Vec<usize> = (0..n).collect();
            for i in (1..n).rev() {
                let j = rng.next() % (i + 1);
                perm.swap(i, j);
            }
            let name = |i: usize| format!("s{:02}$h{:x}", perm[i], perm[i] * 7919);
            let density = 2 + rng.next() % 4;
            let mut items = Vec::new();
            for i in 0..n {
                let mut refs: Vec<String> = Vec::new();
                for j in (i + 1)..n {
                    if rng.next() % density == 0 {
                        refs.push(name(j));
                        if rng.next() % 5 == 0 {
                            refs.push(name(j));
                        }
                    }
                }
                for k in (1..refs.len()).rev() {
                    let j = rng.next() % (k + 1);
                    refs.swap(k, j);
                }
                let mut it = item(&name(i), kind_of(rng.next()), &refs);
                if rng.next() % 9 == 0 {
                    it.order = Some(rcgjs_object::object::SourceOrder {
                        file: "f.rs".to_owned(),
                        line: (rng.next() % 5) as u32,
                        col: (rng.next() % 3) as u32,
                        within: 0,
                    });
                }
                items.push(it);
            }
            let table: BTreeMap<JsName, LinkItem> =
                items.into_iter().map(|it| (it.name.clone(), it)).collect();
            let all: Vec<JsName> = table.keys().cloned().collect();
            let roots: Vec<JsName> = all
                .iter()
                .filter(|_| rng.next() % 3 == 0)
                .cloned()
                .collect();
            for reachable in [all.clone(), closure(&table, &roots)] {
                let new: Vec<&str> = crate::link::order(&table, &reachable)
                    .iter()
                    .map(|i| i.name.as_str())
                    .collect();
                let old: Vec<&str> = old_order(&table, &reachable)
                    .iter()
                    .map(|i| i.name.as_str())
                    .collect();
                assert_eq!(new, old);
                compared += 1;
            }
        }
        assert_eq!(compared, 40000);
    }

    /// On arbitrary graphs, `layout`'s cycles are exactly the strongly connected components
    /// (computed here by transitive closure), every binding is placed once, and every binding
    /// comes after the bindings it names outside its own component.
    #[test]
    fn layout_groups_exactly_the_strongly_connected_components() {
        let mut rng = Rng(0x0dd_ba11_f00d_cafe);
        let mut cyclic = 0;
        for _ in 0..20000 {
            let n = 1 + rng.next() % 12;
            let name = |i: usize| format!("b{i:02}");
            let density = 2 + rng.next() % 6;
            let mut edge = vec![vec![false; n]; n];
            let mut items = Vec::new();
            for i in 0..n {
                let mut refs: Vec<String> = Vec::new();
                for j in 0..n {
                    // an item's own name is never among its refs (JsItem::new)
                    if i != j && rng.next() % density == 0 {
                        refs.push(name(j));
                        edge[i][j] = true;
                    }
                }
                items.push(item(&name(i), ItemKind::Static, &refs));
            }
            let table: BTreeMap<JsName, LinkItem> =
                items.into_iter().map(|it| (it.name.clone(), it)).collect();
            let mut reach = edge.clone();
            for k in 0..n {
                for i in 0..n {
                    for j in 0..n {
                        if reach[i][k] && reach[k][j] {
                            reach[i][j] = true;
                        }
                    }
                }
            }
            let members: HashSet<&JsName> = table.keys().collect();
            let names: Vec<&JsName> = table.keys().collect();
            let steps = layout(&table, &members, &names);
            let index = |item: &LinkItem| item.name.as_str()[1..].parse::<usize>().unwrap();
            let mut position = vec![usize::MAX; n];
            let mut group = vec![usize::MAX; n];
            let mut at = 0;
            for (g, step) in steps.iter().enumerate() {
                let list: Vec<&LinkItem> = match step {
                    Placed::Item(item) => vec![*item],
                    Placed::Cycle(cycle) => cycle.clone(),
                };
                match step {
                    Placed::Item(item) => {
                        let i = index(item);
                        assert!(!reach[i][i], "a binding on a cycle was placed alone");
                    }
                    Placed::Cycle(cycle) => {
                        cyclic += 1;
                        assert!(cycle.len() > 1, "no static names itself in this test");
                    }
                }
                for item in list {
                    let i = index(item);
                    assert_eq!(position[i], usize::MAX, "placed twice");
                    position[i] = at;
                    group[i] = g;
                    at += 1;
                }
            }
            assert!(position.iter().all(|p| *p != usize::MAX), "not placed");
            for i in 0..n {
                for j in 0..n {
                    let same = i == j || (reach[i][j] && reach[j][i]);
                    assert_eq!(group[i] == group[j], same, "component mismatch");
                    if edge[i][j] && !same {
                        assert!(group[j] < group[i], "a binding precedes one it names");
                    }
                }
            }
        }
        assert!(cyclic > 1000);
    }
}
