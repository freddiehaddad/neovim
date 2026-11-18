use super::*;

#[test]
fn shards_cover_every_group_once_and_preserve_order() {
    let targets: Vec<String> = [
        "api",
        "autocmd",
        "core",
        "editor",
        "example_spec.lua",
        "lua",
        "provider",
        "ui",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    let mut covered = BTreeSet::new();
    for index in 1..=3 {
        let shard = TestShard::parse(&format!("{index}/3")).unwrap();
        let selected = shard.select(targets.clone()).unwrap();
        assert_eq!(
            selected,
            targets
                .iter()
                .skip(index - 1)
                .step_by(3)
                .cloned()
                .collect::<Vec<_>>(),
        );
        for target in selected {
            assert!(covered.insert(target), "group assigned more than once");
        }
    }
    assert_eq!(covered, targets.into_iter().collect());
}

#[test]
fn repository_groups_are_partitioned_without_omissions() {
    let root = repo_root();
    let all = lua_test_targets(&root, "functional", &[], None, None).unwrap();
    assert!(!all.is_empty());
    let mut selected = Vec::new();
    for index in 1..=3 {
        let shard = TestShard::parse(&format!("{index}/3")).unwrap();
        let groups = lua_test_targets(&root, "functional", &[], None, Some(shard)).unwrap();
        assert!(!groups.is_empty());
        selected.extend(groups);
    }
    selected.sort();
    assert_eq!(selected, all);
}

#[test]
fn invalid_or_empty_shards_are_rejected() {
    for value in ["", "0/3", "1/0", "4/3", "1", "x/3", "1/2/3", "-1/3"] {
        assert!(TestShard::parse(value).is_err(), "{value}");
    }
    let shard = TestShard::parse("1/3").unwrap();
    assert!(shard.select(vec!["api".into(), "core".into()]).is_err());
    assert!(shard.select(Vec::new()).is_err());
}

#[test]
fn explicit_selection_keeps_its_behavior_and_cannot_be_sharded() {
    let root = Path::new(".");
    let paths = vec!["test/functional/provider".to_string()];
    assert_eq!(
        lua_test_targets(root, "functional", &paths, None, None).unwrap(),
        vec![String::new()],
    );
    assert_eq!(
        lua_test_targets(root, "functional", &[], Some("provider"), None).unwrap(),
        vec!["provider"],
    );
    let shard = Some(TestShard::parse("1/3").unwrap());
    assert!(lua_test_targets(root, "functional", &paths, None, shard).is_err());
    assert!(lua_test_targets(root, "functional", &[], Some("provider"), shard).is_err());
    assert!(lua_test_targets(root, "benchmark", &[], None, shard).is_err());
}
