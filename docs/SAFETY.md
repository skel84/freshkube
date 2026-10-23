# The safety suite

One named set of the read-only and protection guarantees that must never break (#954). Every test in it is a function named `safety_gNN_<rule>`, where `NN` is the guarantee below, so one filter runs all of them across the crates:

```sh
cargo test --workspace --locked safety_g       # the whole suite
cargo test --workspace --locked safety_g05_    # one guarantee
scripts/check-safety-suite.sh                  # fails if a guarantee has no test
```

`scripts/check-safety-suite.sh` lists the compiled tests and fails unless they are exactly the tests this page names, each guarantee has one, none is ignored and the list is not empty. CI runs it as its own step, "Safety suite", after the tests on the macOS check job and the Linux tests job, so a failure reads as "safety". The suite is not a copy: each test is the test that already proves the guarantee, renamed into it, and it sits in the crate whose code it guards. The shared assertion is "nothing was sent": the write framework's recording fake (`freshkube-core`'s `write/testing.rs`) and the fake Kubernetes API's recorder.

**A new write, exec, forward or reveal path adds its guarantee here**, as a `safety_gNN_` test, in the same change. A test belongs in the suite only if it fails when its guard is removed; try that locally before you name one in.

## The guarantees and their tests

### 1. Read-only blocks every write path

- `safety_g01_a_prepared_action_is_still_refused_by_a_read_only_runner` (`crates/freshkube-core/src/write/tests.rs`)
- `safety_g01_a_read_only_destination_blocks_a_promotion_and_asks_the_cluster_nothing` (`crates/freshkube-desktop/src/write_flow/stage_tests.rs`)
- `safety_g01_abort_on_a_read_only_destination_is_blocked_with_no_request` (`crates/freshkube-delivery/src/acts/tests.rs`)
- `safety_g01_read_only_blocks_every_everyday_action_and_makes_no_request` (`crates/freshkube-desktop/src/write_flow/everyday_tests.rs`)
- `safety_g01_read_only_blocks_every_stage_action_and_asks_the_cluster_nothing` (`crates/freshkube-delivery/src/acts/tests.rs`)
- `safety_g01_read_only_blocks_every_tier_and_makes_no_request_at_all` (`crates/freshkube-core/src/write/tests.rs`)
- `safety_g01_read_only_blocks_every_tier_with_its_reason_and_makes_no_request` (`crates/freshkube-desktop/src/write_flow/tests.rs`)
- `safety_g01_read_only_blocks_refresh_with_no_request` (`crates/freshkube-desktop/src/write_flow/stage_tests.rs`)
- `safety_g01_the_level_turning_read_only_after_the_dialog_opened_sends_nothing` (`crates/freshkube-desktop/src/write_flow/tests.rs`)

### 2. Protected needs the typed name; Guarded and Open act as their tier says

- `safety_g02_guarded_scale_asks_then_sends_one_apply_and_is_observed` (`crates/freshkube-core/src/write/tests.rs`)
- `safety_g02_nothing_is_sent_without_the_confirmation_the_level_asks_for` (`crates/freshkube-core/src/write/tests.rs`)
- `safety_g02_open_sends_a_reversible_action_without_a_dialog_but_not_a_spec_change` (`crates/freshkube-core/src/write/tests.rs`)
- `safety_g02_protected_asks_for_the_name_and_a_wrong_or_empty_one_sends_nothing` (`crates/freshkube-desktop/src/write_flow/tests.rs`)
- `safety_g02_protected_scale_sends_only_after_the_name_is_typed` (`crates/freshkube-desktop/src/write_flow/everyday_tests.rs`)
- `safety_g02_protected_sends_once_the_exact_name_is_typed` (`crates/freshkube-desktop/src/write_flow/tests.rs`)
- `safety_g02_protected_takes_the_typed_name_and_the_entry_id_for_several_objects` (`crates/freshkube-core/src/write/tests.rs`)
- `safety_g02_the_exact_stage_name_unlocks_the_send` (`crates/freshkube-desktop/src/write_flow/stage_tests.rs`)

### 3. A team floor is never loosened by a personal setting

- `safety_g03_a_team_file_cannot_widen_what_a_persons_setup_restricts` (`crates/freshkube-core/src/team/tests.rs`)
- `safety_g03_an_own_or_default_level_looser_than_the_floor_shows_the_floor` (`crates/freshkube-desktop/src/desktop/settings/entry/protection_tests.rs`)
- `safety_g03_the_floor_holds_in_both_directions_and_an_unknown_level_is_guarded` (`crates/freshkube-core/src/team/tests.rs`)
- `safety_g03_the_teams_level_is_a_floor_the_inspector_will_not_go_under` (`crates/freshkube-desktop/src/desktop/settings/entry/protection_tests.rs`)
- `safety_g03_the_teams_protection_is_a_floor` (`crates/freshkube-core/src/team/tests.rs`)

### 4. Per-tab levels: a tab's level is its own and never carries across

- `safety_g04_an_open_clusters_shell_can_start_while_a_read_only_cluster_is_shown` (`crates/freshkube-desktop/src/desktop/dock/tests/shells.rs`)
- `safety_g04_leaving_an_open_entry_for_a_stricter_context_re_derives_level_and_write_refusal` (`crates/freshkube-desktop/src/desktop/tabs/cluster_tests.rs`)
- `safety_g04_resources_follow_the_shown_level_while_a_dock_shell_keeps_its_own` (`crates/freshkube-desktop/src/desktop/dock/tests/shells.rs`)

### 5. Dock shells won't start on a read-only cluster; logs still read

- `safety_g05_a_blocked_shell_opens_logs_on_its_own_cluster_and_a_stricter_level_blocks_at_once` (`crates/freshkube-desktop/src/desktop/dock/tests/shells.rs`)
- `safety_g05_a_new_shell_is_blocked_until_its_clusters_level_is_told` (`crates/freshkube-resources/src/shell/tests.rs`)
- `safety_g05_a_read_only_cluster_starts_no_shell` (`crates/freshkube-resources/src/shell/tests.rs`)
- `safety_g05_a_read_only_context_opens_its_shell_tab_idle_and_start_does_nothing` (`crates/freshkube-desktop/src/desktop/dock/tests/shells.rs`)
- `safety_g05_a_shell_blocked_on_the_shown_cluster_offers_logs` (`crates/freshkube-desktop/src/desktop/dock/tests/shells.rs`)

### 6. Exec and port forwarding start only from an explicit action

- `safety_g06_nothing_runs_until_start_which_opens_the_default_container` (`crates/freshkube-resources/src/shell/tests.rs`)
- `safety_g06_opening_a_write_target_from_search_starts_nothing` (`crates/freshkube-desktop/src/desktop/search/tests.rs`)
- `safety_g06_opening_an_object_on_any_tab_starts_no_shell_and_no_forward` (`crates/freshkube-desktop/src/desktop/deep_link/tests.rs`)

### 7. Deep links only navigate

- `safety_g07_a_link_asking_for_a_forward_starts_none` (`crates/freshkube-desktop/src/desktop/deep_link/tests.rs`)
- `safety_g07_a_link_never_opens_a_dialog_a_shell_or_a_forward` (`crates/freshkube-desktop/src/desktop/deep_link/tests.rs`)

### 8. Agents read through the per-cluster gate, off by default, and are read-only

- `safety_g08_a_cluster_without_agent_access_refuses_before_any_request` (`crates/freshkube-core/src/agent_reads/tests.rs`)
- `safety_g08_a_grant_for_another_context_reads_as_off` (`crates/freshkube-core/src/agent_reads/tests.rs`)
- `safety_g08_a_missing_file_grants_nothing_and_an_unparsable_one_grants_nothing_and_logs_once` (`crates/freshkube-agent/tests/saved.rs`)
- `safety_g08_every_agent_read_is_a_get` (`crates/freshkube-core/src/agent_reads/tests.rs`)
- `safety_g08_only_freshkube_tools_are_allowed` (`crates/freshkube-agent/tests/acp.rs`)
- `safety_g08_the_agent_reads_through_the_endpoint_and_a_shell_is_refused` (`crates/freshkube-agent/tests/acp.rs`)
- `safety_g08_turning_access_off_fails_closed_even_when_the_save_or_the_audit_fails` (`crates/freshkube-agent/tests/saved.rs`)

### 9. Secrets: no value in Search, the document or copy without a per-key reveal, forgotten on hide or a new version

- `safety_g09_a_hidden_pane_reads_nothing_and_hides_revealed_values` (`crates/freshkube-resources/src/pane/tests.rs`)
- `safety_g09_a_new_version_hides_revealed_values_again` (`crates/freshkube-resources/src/detail.rs`)
- `safety_g09_a_secret_document_shows_keys_and_sizes_never_values` (`crates/freshkube-core/src/resources/object/tests.rs`)
- `safety_g09_a_secret_is_diffed_and_audited_by_key_names_never_values` (`crates/freshkube-core/src/write/tests.rs`)
- `safety_g09_a_secret_value_is_revealed_only_for_the_same_incarnation` (`crates/freshkube-core/src/resources/object/tests.rs`)
- `safety_g09_searching_secret_names_never_reads_a_secret_document` (`crates/freshkube-desktop/src/desktop/search/tests.rs`)
- `safety_g09_secret_values_stay_hidden_until_one_is_revealed` (`crates/freshkube-resources/src/pane/tests.rs`)

### 10. Operations and maintenance: nothing runs without the explicit confirm flow

- `safety_g10_a_read_only_cluster_refuses_a_restart_and_takes_no_slot` (`crates/freshkube-desktop/src/desktop/tests/operations.rs`)
- `safety_g10_a_read_only_cluster_refuses_review_and_starts_nothing` (`crates/freshkube-desktop/src/screens/operations/tests.rs`)
- `safety_g10_a_run_without_the_previews_access_submits_nothing` (`crates/freshkube-desktop/src/screens/operations/tests.rs`)
- `safety_g10_a_stale_apply_preview_is_refused_and_sends_nothing` (`crates/freshkube-desktop/src/maintenance/tests.rs`)
- `safety_g10_a_stale_restart_preview_is_refused_and_runs_nothing` (`crates/freshkube-desktop/src/desktop/tests/operations.rs`)
- `safety_g10_apply_needs_an_explicit_confirmation_of_the_reviewed_yaml` (`crates/freshkube-desktop/src/maintenance/tests.rs`)
- `safety_g10_confirmation_runs_only_against_the_preview_shown` (`crates/freshkube-desktop/src/desktop/tests/operations.rs`)
- `safety_g10_the_table_chooses_nodes_and_review_still_asks_before_any_step` (`crates/freshkube-desktop/src/screens/operations/tests.rs`)

## Known gaps

- Guarantee 3 is proved where the floor is resolved (`freshkube-core`'s `team`) and where Settings shows and offers levels. The team setup is still example data (nothing is fetched or saved), so no test can show a fetched floor holding down the level a write is sent under; that test joins the suite when it is.
- Guarantee 6 says "never from an agent". The agent can't start an exec or a forward yet (its only tools are the four reads, proved under guarantee 8); a test that the agent never starts one joins the suite when the agent can reach the app (#908).
- At the window's level the example runner sends nothing, so "no write reached the runner" in the link and search tests is read from the runner's scratch audit (`freshkube-example-audit`), which a write would add a line to. The fake cluster's request recorder is used where a runner is built on it (guarantees 1, 2 and 10).
