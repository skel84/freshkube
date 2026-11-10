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

- `safety_g01_a_prepared_action_is_still_refused_by_a_read_only_runner` (`crates/freshkube-core/src/write/tests/levels.rs`)
- `safety_g01_a_read_only_destination_blocks_a_promotion_and_asks_the_cluster_nothing` (`crates/freshkube-desktop/src/write_flow/stage_tests.rs`)
- `safety_g01_abort_on_a_read_only_destination_is_blocked_with_no_request` (`crates/freshkube-delivery/src/acts/tests.rs`)
- `safety_g01_roll_back_on_a_read_only_cluster_sends_no_request_and_a_denied_review_sends_nothing` (`crates/freshkube-core/src/write/tests/rollback.rs`)
- `safety_g01_run_now_on_a_read_only_cluster_sends_no_request_at_all` (`crates/freshkube-core/src/write/tests/run_now.rs`)
- `safety_g01_read_only_blocks_every_everyday_action_and_makes_no_request` (`crates/freshkube-desktop/src/write_flow/everyday_tests.rs`)
- `safety_g01_read_only_blocks_every_stage_action_and_asks_the_cluster_nothing` (`crates/freshkube-delivery/src/acts/tests.rs`)
- `safety_g01_read_only_blocks_every_tier_and_makes_no_request_at_all` (`crates/freshkube-core/src/write/tests/levels.rs`)
- `safety_g01_read_only_blocks_every_tier_with_its_reason_and_makes_no_request` (`crates/freshkube-desktop/src/write_flow/tests.rs`)
- `safety_g01_read_only_blocks_refresh_with_no_request` (`crates/freshkube-desktop/src/write_flow/stage_tests.rs`)
- `safety_g01_the_level_turning_read_only_after_the_dialog_opened_sends_nothing` (`crates/freshkube-desktop/src/write_flow/tests.rs`)

### 2. Protected needs the typed name; Guarded and Open act as their tier says

- `safety_g02_guarded_scale_asks_then_sends_one_apply_and_is_observed` (`crates/freshkube-core/src/write/tests/apply.rs`)
- `safety_g02_nothing_is_sent_without_the_confirmation_the_level_asks_for` (`crates/freshkube-core/src/write/tests/levels.rs`)
- `safety_g02_open_sends_a_reversible_action_without_a_dialog_but_not_a_spec_change` (`crates/freshkube-core/src/write/tests/levels.rs`)
- `safety_g02_roll_back_asks_the_typed_name_when_protected_and_shows_the_diff_even_when_open` (`crates/freshkube-core/src/write/tests/rollback.rs`)
- `safety_g02_protected_asks_for_the_name_and_a_wrong_or_empty_one_sends_nothing` (`crates/freshkube-desktop/src/write_flow/tests.rs`)
- `safety_g02_protected_scale_sends_only_after_the_name_is_typed` (`crates/freshkube-desktop/src/write_flow/everyday_tests.rs`)
- `safety_g02_protected_sends_once_the_exact_name_is_typed` (`crates/freshkube-desktop/src/write_flow/tests.rs`)
- `safety_g02_protected_takes_the_typed_name_and_the_entry_id_for_several_objects` (`crates/freshkube-core/src/write/tests/levels.rs`)
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
- `safety_g08_a_corrupt_or_newer_marker_beside_a_grant_grants_nothing` (`crates/freshkube-agent/tests/saved.rs`)
- `safety_g08_a_missing_file_grants_nothing_and_an_unparsable_one_grants_nothing_and_logs_once` (`crates/freshkube-agent/tests/saved.rs`)
- `safety_g08_a_truncated_corrupt_or_newer_file_grants_nothing_and_a_newer_one_is_not_written` (`crates/freshkube-agent/tests/saved.rs`)
- `safety_g08_every_agent_read_is_a_get` (`crates/freshkube-core/src/agent_reads/tests.rs`)
- `safety_g08_only_freshkube_tools_are_allowed` (`crates/freshkube-agent/tests/acp.rs`)
- `safety_g08_the_agent_reads_through_the_endpoint_and_a_shell_is_refused` (`crates/freshkube-agent/tests/acp.rs`)
- `safety_g08_turning_access_off_fails_closed_even_when_the_save_or_the_audit_fails` (`crates/freshkube-agent/tests/saved.rs`)

### 9. Secrets: no value in Search, the document or copy without a per-key reveal, forgotten on hide or a new version

- `safety_g09_a_hidden_pane_reads_nothing_and_hides_revealed_values` (`crates/freshkube-resources/src/pane/tests/reads.rs`)
- `safety_g09_a_new_version_hides_revealed_values_again` (`crates/freshkube-resources/src/detail.rs`)
- `safety_g09_a_secret_document_shows_keys_and_sizes_never_values` (`crates/freshkube-core/src/resources/object/tests.rs`)
- `safety_g09_a_secret_is_diffed_and_audited_by_key_names_never_values` (`crates/freshkube-core/src/write/tests/audit.rs`)
- `safety_g09_a_secret_value_is_revealed_only_for_the_same_incarnation` (`crates/freshkube-core/src/resources/object/tests.rs`)
- `safety_g09_searching_secret_names_never_reads_a_secret_document` (`crates/freshkube-desktop/src/desktop/search/tests.rs`)
- `safety_g09_secret_values_stay_hidden_until_one_is_revealed` (`crates/freshkube-resources/src/pane/tests/secrets.rs`)

### 10. Operations and maintenance: nothing runs without the explicit confirm flow

- `safety_g10_a_read_only_cluster_refuses_a_restart_and_takes_no_slot` (`crates/freshkube-desktop/src/desktop/tests/operations.rs`)
- `safety_g10_a_read_only_cluster_refuses_review_and_starts_nothing` (`crates/freshkube-desktop/src/screens/operations/tests.rs`)
- `safety_g10_a_run_without_the_previews_access_submits_nothing` (`crates/freshkube-desktop/src/screens/operations/tests.rs`)
- `safety_g10_a_stale_apply_preview_is_refused_and_sends_nothing` (`crates/freshkube-desktop/src/maintenance/tests.rs`)
- `safety_g10_a_stale_restart_preview_is_refused_and_runs_nothing` (`crates/freshkube-desktop/src/desktop/tests/operations.rs`)
- `safety_g10_apply_needs_an_explicit_confirmation_of_the_reviewed_yaml` (`crates/freshkube-desktop/src/maintenance/tests.rs`)
- `safety_g10_confirmation_runs_only_against_the_preview_shown` (`crates/freshkube-desktop/src/desktop/tests/operations.rs`)
- `safety_g10_the_table_chooses_nodes_and_review_still_asks_before_any_step` (`crates/freshkube-desktop/src/screens/operations/tests.rs`)

### 11. Your access only lists namespaces and creates reviews, for the rows in view, and only while shown

- `safety_g11_the_page_only_lists_namespaces_and_creates_reviews_for_rows_in_view` (`crates/freshkube-desktop/src/screens/access/tests.rs`)

### 12. A Run now always confirms, with no countdown or Undo, and Run now and Roll back send only the template that was shown

- `safety_g12_a_template_that_changed_since_the_page_read_it_blocks` (`crates/freshkube-core/src/write/tests/run_now.rs`)
- `safety_g12_a_template_that_changes_after_the_dialog_sends_nothing` (`crates/freshkube-core/src/write/tests/run_now.rs`)
- `safety_g12_roll_back_to_a_template_that_changed_since_it_was_read_sends_nothing` (`crates/freshkube-core/src/write/tests/rollback.rs`)
- `safety_g12_run_now_always_confirms_at_every_level_and_sends_nothing_unasked` (`crates/freshkube-core/src/write/tests/run_now.rs`)
- `safety_g12_run_now_asks_at_every_level_with_a_preview_and_no_typing_or_countdown` (`crates/freshkube-desktop/src/write_flow/everyday_tests.rs`)

### 13. An edit of the YAML is a spec change that sends only what Freshkube owns plus its changes, and never reads or sends a Secret's values

- `safety_g13_a_secret_is_read_as_metadata_and_no_value_is_asked_sent_or_kept` (`crates/freshkube-core/src/write/tests/edit_yaml.rs`)
- `safety_g13_a_secrets_diff_holds_its_metadata_and_the_key_names_it_writes_and_nothing_of_its_values` (`crates/freshkube-core/src/write/diff.rs`)
- `safety_g13_a_new_version_is_never_reviewed_with_the_old_body` (`crates/freshkube-core/src/write/tests/edit_yaml.rs`)
- `safety_g13_a_secret_whose_values_freshkube_owns_is_not_edited` (`crates/freshkube-core/src/write/tests/edit_yaml.rs`)
- `safety_g13_an_edit_after_an_edit_keeps_what_the_first_one_set` (`crates/freshkube-core/src/write/tests/edit_yaml.rs`)
- `safety_g13_an_edit_of_a_secret_never_sends_data_or_string_data_whatever_the_text_holds` (`crates/freshkube-core/src/write/tests/edit_yaml.rs`)
- `safety_g13_an_edit_never_sends_status_managed_fields_or_the_servers_own_fields` (`crates/freshkube-core/src/write/tests/edit_yaml.rs`)
- `safety_g13_read_only_sends_an_edit_nothing_not_even_the_dry_run` (`crates/freshkube-core/src/write/tests/edit_yaml.rs`)
- `safety_g13_the_reply_to_a_secrets_write_is_dropped_of_its_values_whoever_asked` (`crates/freshkube-core/src/write/tests/edit_yaml.rs`)

### 14. Argo CD Refresh sends one annotation, and nothing under `read-only` or when the Application moved

- `safety_g14_argo_refresh_sends_one_annotation_and_nothing_under_read_only_or_when_moved` (`crates/freshkube-core/src/write/everyday/nudges.rs`)
- `safety_g14_refresh_after_one_was_asked_for_sends_nothing_even_after_review_the_new_version` (`crates/freshkube-core/src/write/everyday/nudges.rs`)

### 15. Flux Reconcile sends one annotation, and nothing under `read-only` or when the object moved

- `safety_g15_flux_reconcile_sends_one_annotation_and_nothing_under_read_only_or_when_moved` (`crates/freshkube-core/src/write/everyday/nudges.rs`)
- `safety_g15_reconcile_after_a_suspend_sends_nothing_even_after_review_the_new_version` (`crates/freshkube-core/src/write/everyday/nudges.rs`)

### 16. cert-manager Renew writes only the status conditions, and nothing under `read-only` or when the Certificate moved

- `safety_g16_renew_writes_only_the_status_conditions_and_nothing_under_read_only_or_when_moved` (`crates/freshkube-core/src/write/everyday/nudges.rs`)
- `safety_g16_renew_after_the_conditions_moved_sends_nothing_even_after_review_the_new_version` (`crates/freshkube-core/src/write/everyday/nudges.rs`)

### 17. Delete sends nothing under `read-only`, and deletes only the object the person saw

- `safety_g17_delete_sends_nothing_on_read_only_and_not_for_an_object_that_moved` (`crates/freshkube-core/src/write/tests/delete.rs`)
- `safety_g17_a_read_only_cluster_makes_no_request_for_delete` (`crates/freshkube-desktop/src/desktop/tests/everyday.rs`)

### 18. A destructive action sends nothing during its countdown or after Cancel, a level change, a switch of cluster or context, or leaving the page

- `safety_g18_nothing_is_sent_during_the_countdown_or_after_cancel` (`crates/freshkube-core/src/write/tests/delete.rs`)
- `safety_g18_switching_cluster_during_the_countdown_cancels_it_and_sends_nothing` (`crates/freshkube-desktop/src/desktop/tests/everyday.rs`)
- `safety_g18_a_context_change_during_the_countdown_cancels_it_and_sends_nothing` (`crates/freshkube-desktop/src/desktop/tests/everyday.rs`)
- `safety_g18_leaving_the_page_during_the_countdown_cancels_it_and_sends_nothing` (`crates/freshkube-desktop/src/desktop/tests/everyday.rs`)

### 19. Every diagnostic check says unknown with its source down, and reports no failure on a healthy fixture

- `safety_g19_every_check_with_its_sources_down_says_unknown_and_never_fails` (`crates/freshkube-core/src/diagnostic_runner/harness_tests.rs`)
- `safety_g19_each_source_down_alone_changes_a_check_only_to_unknown` (`crates/freshkube-core/src/diagnostic_runner/harness_tests.rs`)
- `safety_g19_every_check_on_a_healthy_fixture_reports_no_failure` (`crates/freshkube-core/src/diagnostic_runner/harness_tests.rs`)
- `safety_g19_every_check_id_in_the_builders_is_produced_by_the_harness` (`crates/freshkube-core/src/diagnostic_runner/harness_tests.rs`)
- `safety_g19_a_known_gap_names_an_issue_and_still_fails` (`crates/freshkube-core/src/diagnostic_runner/harness_tests.rs`)

An unreachable source is unknown, not a failure, and a healthy node shows no failure. The harness runs every check `DiagnosticSnapshot::evaluate` makes from a snapshot, and builds the snapshots through what the collector decides without I/O (CNI provider from file reads, addon presence, service health), so it does not reach what the collector reads from a live API ([DIAGNOSTICS.md](DIAGNOSTICS.md#the-harness)). A new check is run by it without an edit. The first and second tests are the source-down case, the third the healthy case; the fourth fails when a check id is written that no scenario reaches; the fifth keeps each known gap tied to an issue and fails once it stops failing.

### 20. A database session never writes and never leaves its read-only transaction

The explorer's and the Query tab's session (`freshkube-db`, [database-explorer.md](features/database-explorer.md)). The database refuses a write, and nothing is retried. The statement checks are a guard and not a sandbox: until a reader role arrives (E4) every live session is the application's own user, so what a read-only transaction lets through but still acts on the server (maintenance, other sessions, files) is refused by name before it is sent.

- `safety_g20_a_write_is_refused_by_the_read_only_transaction_alone_and_nothing_is_applied` (`crates/freshkube-db/src/safety.rs`)
- `safety_g20_a_server_that_does_not_hold_read_only_is_never_used` (`crates/freshkube-db/src/safety.rs`)
- `safety_g20_statements_that_leave_the_transaction_or_change_the_session_are_not_sent` (`crates/freshkube-db/src/safety.rs`)
- `safety_g20_maintenance_and_functions_that_act_on_the_server_are_not_sent` (`crates/freshkube-db/src/safety.rs`)
- `safety_g20_sql_built_at_run_time_or_spelled_in_escapes_is_not_sent` (`crates/freshkube-db/src/safety.rs`)
- `safety_g20_a_plan_never_analyzes` (`crates/freshkube-db/src/safety.rs`)
- `safety_g20_every_request_ends_rolled_back_with_its_locks_let_go_and_nothing_is_committed` (`crates/freshkube-db/src/safety.rs`)

### 21. A Teleport child belongs to one explorer tab, runs only from Explore on loopback, never signs in for anyone, and carries no secret

- `safety_g21_closing_the_tab_kills_the_teleport_child` (`crates/freshkube-desktop/src/screens/databases/explore/tests.rs`)
- `safety_g21_quitting_the_app_ends_the_teleport_child` (`crates/freshkube-desktop/src/screens/databases/explore/tests.rs`)
- `safety_g21_the_child_dies_with_its_proxy` (`crates/freshkube-db/src/teleport/tests.rs`)
- `safety_g21_a_proxy_that_listens_beyond_loopback_is_killed` (`crates/freshkube-db/src/teleport/tests.rs`)
- `safety_g21_not_signed_in_is_told_without_running_a_login` (`crates/freshkube-db/src/teleport/tests.rs`)
- `safety_g21_the_childs_arguments_hold_no_secret` (`crates/freshkube-db/src/teleport/tests.rs`)
- `safety_g21_the_child_gets_only_the_environment_tsh_needs` (`crates/freshkube-db/src/teleport/tests.rs`)

### 22. What is offered asks reviews only for the selection's actions, once the selection has rested and only while shown, and an answer never stands in for the gate

- `safety_g22_only_the_selected_rows_actions_are_asked_about` (`crates/freshkube-resources/src/screen/tests/offers.rs`)
- `safety_g22_a_held_arrow_asks_about_the_row_it_stops_on_and_a_hidden_page_asks_nothing` (`crates/freshkube-resources/src/screen/tests/offers.rs`)
- `safety_g22_on_a_live_cluster_only_the_last_selection_is_reviewed_and_a_hide_stops_the_rest` (`crates/freshkube-resources/src/screen/tests/offers.rs`)
- `safety_g22_a_selection_change_behind_four_reviews_in_flight_asks_only_the_new_row` (`crates/freshkube-resources/src/screen/tests/offers.rs`)
- `safety_g22_a_hide_drops_what_is_queued_and_in_flight_and_nothing_lands` (`crates/freshkube-resources/src/screen/tests/offers.rs`)
- `safety_g22_an_allowed_answer_on_the_page_does_not_skip_the_dialogs_review` (`crates/freshkube-desktop/src/desktop/tests/everyday.rs`)

### 23. A TLS Secret is read only on request, `tls.key` never leaves core, and no Secret is read where a Certificate's status answers

The Certificates page (`freshkube-core`'s `resources/traffic/certs/`, [traffic.md](features/traffic.md#certificates-n4)). It lists and gets objects, and reads no Secret to draw its rows: where a cert-manager Certificate keeps the Secret, its names and expiry come from that Certificate's status, and core makes no request for the Secret at all. A Secret that no Certificate that was read keeps is offered for reading only when every Certificate could be read; with that list refused, cut or its discovery failed, Read Secret is withheld and the row says why. Any other Secret is read by one GET of its name, only when the person presses Read Secret, and only the parsed facts of `tls.crt` leave core (subject, names, issuer, not-before, not-after); the result type has no field for `tls.key`, for the PEM or for any other key, and the decoded values are wiped before it returns. The facts stay in the page's memory, survive the shell's automatic refreshes of the same source (they are not checked against later versions of the Secret: that would take a Secret read nobody asked for), and go with a hidden page or a change of source. A refused read is Unknown, never "wrong".

- `safety_g23_tls_key_never_leaves_core` (`crates/freshkube-core/src/resources/traffic/certs/tests.rs`)
- `safety_g23_the_page_read_asks_for_no_secret_and_a_managed_secret_is_never_requested` (`crates/freshkube-core/src/resources/traffic/certs/tests.rs`)
- `safety_g23_a_secret_is_read_only_by_name_and_with_one_get` (`crates/freshkube-core/src/resources/traffic/certs/tests.rs`)
- `safety_g23_a_refused_or_missing_secret_is_unknown_or_not_found_never_wrong` (`crates/freshkube-core/src/resources/traffic/certs/tests.rs`)
- `safety_g23_nothing_reads_a_secret_until_the_control_is_pressed` (`crates/freshkube-desktop/src/screens/certificates/tests.rs`)

## Known gaps

- Guarantee 3 is proved where the floor is resolved (`freshkube-core`'s `team`) and where Settings shows and offers levels. The team setup is still example data (nothing is fetched or saved), so no test can show a fetched floor holding down the level a write is sent under; that test joins the suite when it is.
- Guarantee 6 says "never from an agent". The agent can't start an exec or a forward yet (its only tools are the four reads, proved under guarantee 8); a test that the agent never starts one joins the suite when the agent can reach the app (#908).
- At the window's level the example runner sends nothing, so "no write reached the runner" in the link and search tests is read from the runner's scratch audit (`freshkube-example-audit`), which a write would add a line to. The fake cluster's request recorder is used where a runner is built on it (guarantees 1, 2 and 10).
- Guarantee 21 has a Windows gap: the Teleport child has no parent-death watcher there (macOS and Linux have one, tested by `the_group_ends_when_the_parent_dies_without_ending_it`), so a crash or a force quit can leave `tsh proxy db` running until its Teleport certificate expires; a clean quit still ends it. A job object, as `freshkube-agent` uses for the agent, would close it.
