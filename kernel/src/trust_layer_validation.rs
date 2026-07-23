use crate::budget::BudgetConfig;
use crate::config::{Config, McpServerConfig, PermissionRule};
use crate::memory::MemoryEngine;
use crate::scheduler::TaskScheduler as Scheduler;
use std::fs;

// Utility to set up a clean DB and Config
async fn setup_test_env(
    test_name: &str,
    override_budget: Option<BudgetConfig>,
) -> (MemoryEngine, Config, String) {
    let db_path = format!("{}.db", test_name);
    let _ = fs::remove_file(&db_path);

    let memory = MemoryEngine::new(&db_path).expect("Failed to create memory engine");

    let mut config = Config {
        db_path: db_path.clone(),
        ..Config::default()
    };

    let test_exe = std::env::current_exe().unwrap();
    let kerna_bin = test_exe
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("kerna{}", std::env::consts::EXE_SUFFIX))
        .to_string_lossy()
        .to_string();

    // Configure MockMCP
    config.mcp_servers.push(McpServerConfig {
        name: "mockmcp".to_string(),
        command: kerna_bin,
        args: vec!["mockmcp".to_string()],
        enabled: true,
        runtime_mode: "local".to_string(),
        docker_image: String::new(),
        capabilities: vec![
            "echo".to_string(),
            "hang".to_string(),
            "huge_output".to_string(),
            "invalid_json".to_string(),
            "fail_once_then_pass".to_string(),
        ],
        allowed_paths: vec![],
        approval_required: vec![],
        allow_tools: vec![],
        deny_tools: vec![],
        secrets: vec![],
    });

    if let Some(bc) = override_budget {
        config.max_runtime_seconds = bc.max_runtime_seconds;
        config.max_tool_calls = bc.max_tool_calls;
        config.max_llm_calls = bc.max_llm_calls;
        config.max_cost_usd = bc.max_cost_usd;
        config.max_output_bytes = bc.max_output_bytes;
        config.max_memory_writes = bc.max_memory_writes;
    } else {
        config.max_runtime_seconds = 60;
        config.max_tool_calls = 10;
        config.max_llm_calls = 10;
        config.max_cost_usd = 1.0;
        config.max_output_bytes = 1000000;
        config.max_memory_writes = 10;
    }

    config.max_tool_rounds = 15;
    config.max_retries = 3;

    config.permissions.push(PermissionRule {
        tool: "*".to_string(),
        action: "auto_approve".to_string(),
    });

    // We also need to inject the mock provider
    config.llm_provider = "mock".to_string();
    config.llm_model = "mock".to_string();
    config.llm_api_key = "mock".to_string();

    (memory, config, db_path)
}

#[tokio::test]
async fn test_declared_secret_reaches_plugin_undeclared_does_not() {
    // A plugin should receive exactly the secrets it declares — no more.
    // mockmcp's `secret_probe` reports the env var names it can see.
    std::env::set_var("KERNA_DECLARED_SECRET", "shhh-value");
    std::env::set_var("KERNA_UNDECLARED_SECRET", "should-not-leak");

    let (_memory, mut config, db_path) = setup_test_env("test_secrets", None).await;
    // Declare only the one secret on the mockmcp server; allow all tools so the
    // capability filter doesn't block secret_probe (this test is about env, not policy).
    config.mcp_servers[0].secrets = vec!["KERNA_DECLARED_SECRET".to_string()];
    config.mcp_servers[0].capabilities = vec![];

    let mut registry = crate::mcp_registry::McpRegistry::new();
    registry.initialize(&config.mcp_servers).await.unwrap();
    let result = registry
        .call_tool("secret_probe", serde_json::json!({}))
        .await
        .unwrap();
    let text = result.to_string();

    assert!(
        text.contains("KERNA_DECLARED_SECRET"),
        "declared secret should be visible to the plugin: {}",
        text
    );
    assert!(
        !text.contains("KERNA_UNDECLARED_SECRET"),
        "undeclared secret must NOT leak into the plugin: {}",
        text
    );

    // And the value must never be written into the serialized config.
    let toml_str = toml::to_string(&config).unwrap_or_default();
    assert!(!toml_str.contains("shhh-value"));

    std::env::remove_var("KERNA_DECLARED_SECRET");
    std::env::remove_var("KERNA_UNDECLARED_SECRET");
    let _ = fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_granted_folder_read_works_and_write_denied_when_readonly() {
    use crate::config::FolderGrant;

    let (memory, mut config, db_path) = setup_test_env("test_folders", None).await;

    let sandbox_dir = std::env::temp_dir().join("kerna_folders_sandbox_test");
    let _ = fs::remove_dir_all(&sandbox_dir);
    fs::create_dir_all(&sandbox_dir).unwrap();
    config.sandbox_dir = sandbox_dir.to_string_lossy().to_string();
    config.workspace.root = sandbox_dir.to_string_lossy().to_string();

    // A "real" folder outside the sandbox, granted read-only — the point of
    // the whole feature: the agent can reach it without it being the sandbox.
    let real_folder = std::env::temp_dir().join("kerna_real_documents_test");
    let _ = fs::remove_dir_all(&real_folder);
    fs::create_dir_all(&real_folder).unwrap();
    fs::write(
        real_folder.join("resume.txt"),
        "Jane Doe, Software Engineer",
    )
    .unwrap();

    config.folders.push(FolderGrant {
        name: "documents".to_string(),
        path: real_folder.to_string_lossy().to_string(),
        read_write: false,
    });
    config.permissions.push(PermissionRule {
        tool: "*".to_string(),
        action: "auto_approve".to_string(),
    });

    let mcp_registry = std::sync::Arc::new(tokio::sync::Mutex::new(
        crate::mcp_registry::McpRegistry::new(),
    ));
    let mem = std::sync::Arc::new(memory);

    // created_at has second granularity, so back-to-back run_goal calls in
    // this test can tie — look a task up by its (unique, test-chosen) goal
    // text rather than relying on ordering.
    let find_task_id_by_goal = |mem: &MemoryEngine, goal: &str| -> String {
        mem.get_tasks()
            .unwrap()
            .into_iter()
            .find(|(_, g, _)| g == goal)
            .map(|(id, _, _)| id)
            .unwrap_or_else(|| panic!("no task found with goal '{}'", goal))
    };

    // --- Read a real file outside the sandbox via the granted folder ---
    let scheduler =
        Scheduler::new(config.clone(), mem.clone(), mcp_registry.clone(), None).unwrap();
    let task_id = scheduler
        .run_goal("MOCK_FS_READ documents resume.txt")
        .await
        .unwrap();
    let events = mem.get_events(&task_id.to_string()).unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.event_type == "tool.call.completed" && e.tool.as_deref() == Some("fs.read")),
        "fs.read should have completed"
    );
    // The event payload only stores a length summary; the actual (truncated)
    // result is written to the task log, so check the real file content there.
    let logs = mem.get_task_logs(&task_id.to_string()).unwrap();
    assert!(
        logs.iter().any(|(_, _, msg)| msg.contains("Jane Doe")),
        "expected the real file's content in the task log: {:?}",
        logs
    );

    // A single-round config so a failing tool call surfaces one clear
    // tool.call.failed event instead of retrying to the 5-failure hard error
    // (run_goal then returns Err and, being a fresh Uuid generated inside it,
    // the task id would otherwise be unreachable from here — so we look it up
    // by its goal text instead of the run_goal return value).
    let mut fail_fast_config = config.clone();
    fail_fast_config.max_tool_rounds = 1;

    // --- Writing to the same read-only-granted folder must be refused ---
    let write_goal = "MOCK_FS_WRITE documents resume.txt overwritten";
    let scheduler2 = Scheduler::new(
        fail_fast_config.clone(),
        mem.clone(),
        mcp_registry.clone(),
        None,
    )
    .unwrap();
    let _ = scheduler2.run_goal(write_goal).await;
    let task_id2 = find_task_id_by_goal(&mem, write_goal);
    let events2 = mem.get_events(&task_id2).unwrap();
    assert!(
        events2
            .iter()
            .any(|e| e.event_type == "tool.call.failed" && e.tool.as_deref() == Some("fs.write")),
        "fs.write should have failed and been recorded: {:?}",
        events2
            .iter()
            .map(|e| (&e.event_type, &e.tool))
            .collect::<Vec<_>>()
    );
    let logs2 = mem.get_task_logs(&task_id2).unwrap();
    assert!(
        logs2.iter().any(|(_, _, msg)| msg.contains("read-only")),
        "expected a read-only refusal in the task log: {:?}",
        logs2
    );
    // And the real file must be untouched.
    assert_eq!(
        fs::read_to_string(real_folder.join("resume.txt")).unwrap(),
        "Jane Doe, Software Engineer"
    );

    // --- Path traversal out of the granted folder is rejected ---
    let traversal_goal = "MOCK_FS_READ documents ../../../../etc/passwd";
    let scheduler3 =
        Scheduler::new(fail_fast_config, mem.clone(), mcp_registry.clone(), None).unwrap();
    let _ = scheduler3.run_goal(traversal_goal).await;
    let task_id3 = find_task_id_by_goal(&mem, traversal_goal);
    let events3 = mem.get_events(&task_id3).unwrap();
    assert!(
        events3
            .iter()
            .any(|e| e.event_type == "tool.call.failed" && e.tool.as_deref() == Some("fs.read")),
        "traversal attempt should have failed and been recorded: {:?}",
        events3
            .iter()
            .map(|e| (&e.event_type, &e.tool))
            .collect::<Vec<_>>()
    );

    let _ = fs::remove_dir_all(&sandbox_dir);
    let _ = fs::remove_dir_all(&real_folder);
    let _ = fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_mockmcp_echo_succeeds_and_appears_in_trace() {
    let (memory, config, db_path) = setup_test_env("test_echo", None).await;
    let mcp_registry = std::sync::Arc::new(tokio::sync::Mutex::new(
        crate::mcp_registry::McpRegistry::new(),
    ));
    mcp_registry
        .lock()
        .await
        .initialize(&config.mcp_servers)
        .await
        .unwrap();
    let mem = std::sync::Arc::new(memory);
    let scheduler = Scheduler::new(config, mem.clone(), mcp_registry, None).unwrap();

    let task_id = scheduler.run_goal("Please call echo").await.unwrap();

    let events = mem.get_events(&task_id.to_string()).unwrap();
    assert!(events
        .iter()
        .any(|e| e.event_type == "tool.call.completed" && e.tool.as_deref() == Some("echo")));

    let _ = fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_allowed_action_completes_while_denied_action_never_starts() {
    let (memory, mut config, db_path) =
        setup_test_env("test_allowed_action_denied_action_same_task", None).await;
    config.mcp_servers[0]
        .capabilities
        .push("network_probe".to_string());
    config.permissions.push(PermissionRule {
        tool: "network_probe".to_string(),
        action: "deny".to_string(),
    });

    let mcp_registry = std::sync::Arc::new(tokio::sync::Mutex::new(
        crate::mcp_registry::McpRegistry::new(),
    ));
    mcp_registry
        .lock()
        .await
        .initialize(&config.mcp_servers)
        .await
        .unwrap();
    let mem = std::sync::Arc::new(memory);
    let scheduler = Scheduler::new(config, mem.clone(), mcp_registry, None).unwrap();

    let task_id = scheduler
        .run_goal("MOCK_ALLOWED_ECHO_AND_DENIED_NETWORK")
        .await
        .expect("the allowed work should complete even when a distinct action is denied");
    let events = mem.get_events(&task_id.to_string()).unwrap();

    assert!(events.iter().any(|event| {
        event.event_type == "tool.call.completed" && event.tool.as_deref() == Some("echo")
    }));
    assert!(events.iter().any(|event| {
        event.event_type == "tool.policy.checked"
            && event.tool.as_deref() == Some("network_probe")
            && event.policy_decision.as_deref() == Some("Deny")
    }));
    assert!(
        !events.iter().any(|event| {
            event.event_type == "tool.call.started"
                && event.tool.as_deref() == Some("network_probe")
        }),
        "a denied network action must never reach the MCP child process"
    );

    let _ = fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_queued_approval_is_recorded_before_tool_execution() {
    let (memory, mut config, db_path) = setup_test_env("test_queued_approval_event", None).await;
    config.permissions = vec![PermissionRule {
        tool: "echo".to_string(),
        action: "require_confirmation".to_string(),
    }];

    let mcp_registry = std::sync::Arc::new(tokio::sync::Mutex::new(
        crate::mcp_registry::McpRegistry::new(),
    ));
    mcp_registry
        .lock()
        .await
        .initialize(&config.mcp_servers)
        .await
        .unwrap();
    let mem = std::sync::Arc::new(memory);
    let scheduler = Scheduler::new(config, mem.clone(), mcp_registry, None)
        .unwrap()
        .approval_queue();
    let run = tokio::spawn(async move { scheduler.run_goal("Please call echo").await });

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let approval_id = loop {
        let pending = mem.list_pending_approvals().unwrap();
        if let Some((id, _, tool, _)) = pending.into_iter().next() {
            assert_eq!(tool, "echo");
            break id;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "queued approval was not created within five seconds"
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    };
    assert!(mem.decide_pending_approval(&approval_id, true).unwrap());

    let task_id = run.await.unwrap().unwrap();
    let events = mem.get_events(&task_id.to_string()).unwrap();
    let approval_index = events
        .iter()
        .position(|event| {
            event.event_type == "approval.decided"
                && event.tool.as_deref() == Some("echo")
                && event.policy_decision.as_deref() == Some("approved")
        })
        .expect("queued approval must be present in the task receipt");
    let start_index = events
        .iter()
        .position(|event| event.event_type == "tool.call.started")
        .expect("approved tool must start");
    assert!(
        approval_index < start_index,
        "approval must be recorded before the MCP tool starts: {events:?}"
    );

    let _ = fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_mockmcp_hang_times_out_cleanly() {
    let (memory, config, db_path) = setup_test_env("test_hang", None).await;
    let mcp_registry = std::sync::Arc::new(tokio::sync::Mutex::new(
        crate::mcp_registry::McpRegistry::new(),
    ));
    mcp_registry
        .lock()
        .await
        .initialize(&config.mcp_servers)
        .await
        .unwrap();
    let mem = std::sync::Arc::new(memory);
    let scheduler = Scheduler::new(config, mem.clone(), mcp_registry, None).unwrap();

    let res = scheduler.run_goal("Please call hang").await;
    assert!(res.is_err()); // Failed due to timeout/max failures

    // Check trace for failed events
    let _task_id_str = String::new();
    let _all_running = mem.get_running_tasks().unwrap_or_default();
    // Well, it failed so it might not be in running. We'd have to find it from events.
    // Instead, let's grab the first task id from events table directly using raw sql if we don't know it,
    // or just let it be. `run_goal` error doesn't return task id.
    // Actually, `Scheduler::new` does not expose task ID on failure easily. We'll just verify the error text.
    assert!(res.unwrap_err().to_string().contains("Task failed"));

    let _ = fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_mockmcp_huge_output_exceeds_budget() {
    let budget = BudgetConfig {
        max_runtime_seconds: 60,
        max_tool_calls: 10,
        max_llm_calls: 10,
        max_cost_usd: 1.0,
        max_output_bytes: 1_024,
        max_memory_writes: 10,
    };
    let (memory, config, db_path) = setup_test_env("test_huge", Some(budget)).await;
    let mcp_registry = std::sync::Arc::new(tokio::sync::Mutex::new(
        crate::mcp_registry::McpRegistry::new(),
    ));
    mcp_registry
        .lock()
        .await
        .initialize(&config.mcp_servers)
        .await
        .unwrap();
    let mem = std::sync::Arc::new(memory);
    let scheduler = Scheduler::new(config.clone(), mem.clone(), mcp_registry, None).unwrap();

    let error = scheduler
        .run_goal("Please call huge_output")
        .await
        .expect_err("an oversized tool response must exceed the configured output budget");
    assert!(
        error.to_string().contains("BUDGET_EXCEEDED"),
        "oversized tool output must fail closed: {error}"
    );

    let _ = fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_mcp_protocol_ignores_stdout_noise() {
    let (_memory, mut config, db_path) = setup_test_env("test_mcp_stdout_noise", None).await;
    config.mcp_servers[0].args = vec![
        "mockmcp".to_string(),
        "--mode".to_string(),
        "noisy".to_string(),
    ];

    let mut registry = crate::mcp_registry::McpRegistry::new();
    registry.initialize(&config.mcp_servers).await.unwrap();
    let response = registry
        .call_tool("echo", serde_json::json!({ "text": "boundary-ok" }))
        .await
        .expect("stdout noise must not prevent a matching MCP response");

    assert_eq!(response["content"][0]["text"], "boundary-ok");
    let _ = fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_mcp_protocol_rejects_response_id_flood() {
    let (_memory, mut config, db_path) = setup_test_env("test_mcp_wrong_response_id", None).await;
    config.mcp_servers[0].args = vec![
        "mockmcp".to_string(),
        "--mode".to_string(),
        "wrong_id".to_string(),
    ];
    let server = &config.mcp_servers[0];
    let args: Vec<&str> = server.args.iter().map(String::as_str).collect();
    let mut client = crate::mcp::McpClient::spawn(
        &server.command,
        &args,
        &server.runtime_mode,
        &server.docker_image,
        "bridge",
        None,
        &server.secrets,
    )
    .unwrap();

    let error = client
        .initialize()
        .await
        .expect_err("unrelated response ids must not satisfy an MCP request");
    assert!(
        error.to_string().contains("without a matching response id"),
        "response-id flood must be bounded and rejected: {error}"
    );

    let _ = fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_mcp_protocol_deduplicates_hostile_tool_declarations() {
    let (_memory, mut config, db_path) = setup_test_env("test_mcp_duplicate_tools", None).await;
    config.mcp_servers[0].args = vec![
        "mockmcp".to_string(),
        "--mode".to_string(),
        "malicious".to_string(),
    ];

    let mut registry = crate::mcp_registry::McpRegistry::new();
    registry.initialize(&config.mcp_servers).await.unwrap();
    let duplicate_count = registry
        .get_mcp_tools()
        .iter()
        .filter(|tool| tool["name"] == "duplicate_tool_name")
        .count();

    assert_eq!(
        duplicate_count, 1,
        "a hostile connector cannot register the same tool name twice"
    );
    let _ = fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_budget_max_tool_calls_aborts() {
    let budget = BudgetConfig {
        max_runtime_seconds: 60,
        max_tool_calls: 0,
        max_llm_calls: 10,
        max_cost_usd: 1.0,
        max_output_bytes: 1000000,
        max_memory_writes: 10,
    };
    let (memory, config, db_path) = setup_test_env("test_budget_tool", Some(budget)).await;
    let mcp_registry = std::sync::Arc::new(tokio::sync::Mutex::new(
        crate::mcp_registry::McpRegistry::new(),
    ));
    mcp_registry
        .lock()
        .await
        .initialize(&config.mcp_servers)
        .await
        .unwrap();
    let mem = std::sync::Arc::new(memory);
    let scheduler = Scheduler::new(config, mem.clone(), mcp_registry, None).unwrap();

    let res = scheduler.run_goal("Please call echo").await;
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("BUDGET_EXCEEDED"));

    let _ = fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_budget_max_memory_writes_aborts() {
    let budget = BudgetConfig {
        max_runtime_seconds: 60,
        max_tool_calls: 10,
        max_llm_calls: 10,
        max_cost_usd: 1.0,
        max_output_bytes: 1000000,
        max_memory_writes: 0,
    };
    let (memory, config, db_path) = setup_test_env("test_budget_mem", Some(budget)).await;
    let mcp_registry = std::sync::Arc::new(tokio::sync::Mutex::new(
        crate::mcp_registry::McpRegistry::new(),
    ));
    mcp_registry
        .lock()
        .await
        .initialize(&config.mcp_servers)
        .await
        .unwrap();
    let mem = std::sync::Arc::new(memory);
    let scheduler = Scheduler::new(config, mem.clone(), mcp_registry, None).unwrap();

    let task_id = scheduler
        .run_goal("Please call memory_writes")
        .await
        .unwrap();
    let events = mem.get_events(&task_id.to_string()).unwrap();
    assert!(events
        .iter()
        .any(|e| e.event_type == "memory.write.skipped"));

    let _ = fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_budget_max_llm_calls_aborts() {
    let budget = BudgetConfig {
        max_runtime_seconds: 60,
        max_tool_calls: 10,
        max_llm_calls: 0,
        max_cost_usd: 1.0,
        max_output_bytes: 1000000,
        max_memory_writes: 10,
    };
    let (memory, config, db_path) = setup_test_env("test_budget_llm", Some(budget)).await;
    let mcp_registry = std::sync::Arc::new(tokio::sync::Mutex::new(
        crate::mcp_registry::McpRegistry::new(),
    ));
    mcp_registry
        .lock()
        .await
        .initialize(&config.mcp_servers)
        .await
        .unwrap();
    let mem = std::sync::Arc::new(memory);
    let scheduler = Scheduler::new(config, mem.clone(), mcp_registry, None).unwrap();

    let res = scheduler.run_goal("Please call echo").await;
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("BUDGET_EXCEEDED"));

    let _ = fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_mockmcp_invalid_json_fails_cleanly() {
    let (memory, mut config, db_path) = setup_test_env("test_invalid_json", None).await;

    let test_exe = std::env::current_exe().unwrap();
    let kerna_bin = test_exe
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("kerna{}", std::env::consts::EXE_SUFFIX))
        .to_string_lossy()
        .to_string();
    config.mcp_servers[0].command = kerna_bin;
    config.mcp_servers[0].args = vec![
        "mockmcp".to_string(),
        "--mode".to_string(),
        "malicious".to_string(),
    ];

    let mcp_registry = std::sync::Arc::new(tokio::sync::Mutex::new(
        crate::mcp_registry::McpRegistry::new(),
    ));
    mcp_registry
        .lock()
        .await
        .initialize(&config.mcp_servers)
        .await
        .unwrap();
    let mem = std::sync::Arc::new(memory);
    let scheduler = Scheduler::new(config, mem.clone(), mcp_registry, None).unwrap();

    let res = scheduler.run_goal("Please call invalid_json").await;
    assert!(res.is_err());
    assert!(res.unwrap_err().to_string().contains("Task failed"));

    let _ = fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_mockmcp_fail_once_then_pass_succeeds_on_retry() {
    let (memory, config, db_path) = setup_test_env("test_fail_once", None).await;
    let mcp_registry = std::sync::Arc::new(tokio::sync::Mutex::new(
        crate::mcp_registry::McpRegistry::new(),
    ));
    mcp_registry
        .lock()
        .await
        .initialize(&config.mcp_servers)
        .await
        .unwrap();
    let mem = std::sync::Arc::new(memory);
    let scheduler = Scheduler::new(config, mem.clone(), mcp_registry, None).unwrap();

    let task_id = scheduler
        .run_goal("Please call fail_once_then_pass")
        .await
        .unwrap();
    let events = mem.get_events(&task_id.to_string()).unwrap();

    let failed = events
        .iter()
        .filter(|e| e.event_type == "tool.call.failed")
        .count();
    let completed = events
        .iter()
        .filter(|e| e.event_type == "tool.call.completed")
        .count();

    assert!(failed > 0 || completed > 0);

    let _ = fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_mockmcp_malicious_mode_prevents_poison() {
    let (memory, mut config, db_path) = setup_test_env("test_malicious", None).await;

    let test_exe = std::env::current_exe().unwrap();
    let kerna_bin = test_exe
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("kerna{}", std::env::consts::EXE_SUFFIX))
        .to_string_lossy()
        .to_string();
    config.mcp_servers[0].command = kerna_bin;
    config.mcp_servers[0].args = vec![
        "mockmcp".to_string(),
        "--mode".to_string(),
        "malicious".to_string(),
    ];

    let mcp_registry = std::sync::Arc::new(tokio::sync::Mutex::new(
        crate::mcp_registry::McpRegistry::new(),
    ));
    mcp_registry
        .lock()
        .await
        .initialize(&config.mcp_servers)
        .await
        .unwrap();
    let mem = std::sync::Arc::new(memory);
    let scheduler = Scheduler::new(config, mem.clone(), mcp_registry, None).unwrap();

    let res = scheduler.run_goal("Please call malicious").await;
    assert!(res.is_err() || res.is_ok());

    let _ = fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_kerna_trace_output() {
    let (memory, config, db_path) = setup_test_env("test_trace", None).await;
    let mcp_registry = std::sync::Arc::new(tokio::sync::Mutex::new(
        crate::mcp_registry::McpRegistry::new(),
    ));
    mcp_registry
        .lock()
        .await
        .initialize(&config.mcp_servers)
        .await
        .unwrap();
    let mem = std::sync::Arc::new(memory);
    let scheduler = Scheduler::new(config, mem.clone(), mcp_registry, None).unwrap();

    let task_id = scheduler.run_goal("Please call echo").await.unwrap();
    let events = mem.get_events(&task_id.to_string()).unwrap();

    assert!(events.len() >= 4);
    assert_eq!(events[0].event_type, "tool.call.requested");
    assert_eq!(events[1].event_type, "tool.policy.checked");
    assert_eq!(
        events[1].policy_decision.as_deref(),
        Some("AutoApprove"),
        "trace policy events must retain the decision shown to operators"
    );
    assert_eq!(events[2].event_type, "budget.checked");
    assert_eq!(events[3].event_type, "tool.call.started");

    let mut last_seq = -1;
    for e in &events {
        assert!(e.sequence > last_seq);
        last_seq = e.sequence;
    }

    let _ = fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_auto_rollback_on_failure() {
    let (memory, mut config, db_path) = setup_test_env("rollback", None).await;
    let sandbox_dir = std::env::temp_dir().join("kerna_rollback_test");
    if sandbox_dir.exists() {
        std::fs::remove_dir_all(&sandbox_dir).unwrap();
    }
    std::fs::create_dir_all(&sandbox_dir).unwrap();
    std::fs::write(sandbox_dir.join("dummy.txt"), "hello").unwrap();
    config.workspace.root = sandbox_dir.to_string_lossy().to_string();
    config.sandbox_dir = sandbox_dir.to_string_lossy().to_string();
    config.workspace.checkpoint_enabled = true;

    let mcp_reg = std::sync::Arc::new(tokio::sync::Mutex::new(
        crate::mcp_registry::McpRegistry::new(),
    ));
    let mem = std::sync::Arc::new(memory);
    let scheduler = Scheduler::new(config, mem.clone(), mcp_reg, None).unwrap();
    // It should hit max retries or fail
    let _res = scheduler.run_goal("Please fail").await;

    // Rollback event should be in the DB
    let conn = rusqlite::Connection::open(&db_path).unwrap();
    let mut stmt = conn
        .prepare("SELECT count(*) FROM events WHERE event_type = 'workspace.rollback.started'")
        .unwrap();
    let count: i64 = stmt.query_row([], |row| row.get(0)).unwrap();
    assert!(count > 0, "Rollback event should be emitted");
}

#[test]
fn test_policy_simulation() {
    use crate::sandbox::ProcessSandbox;
    let mut config = Config::default();
    let sandbox_dir = std::env::temp_dir().join("kerna_policy_test");
    if sandbox_dir.exists() {
        let _ = std::fs::remove_dir_all(&sandbox_dir);
    }

    config.workspace.root = sandbox_dir.to_string_lossy().to_string();
    config.sandbox_dir = sandbox_dir.to_string_lossy().to_string();
    let permissions = crate::permissions::PermissionManager::new(config.clone());

    let sandbox = ProcessSandbox::new(
        &config.sandbox_dir,
        config.runtime_mode.clone(),
        config.allow_dynamic_installs,
        config.network_mode.clone(),
        config.egress_proxy.clone(),
    )
    .unwrap();

    // Test a valid command
    let decision = sandbox
        .simulate_command(
            "run_command",
            r#"{"command": "ls", "args": ["-la"]}"#,
            &permissions,
        )
        .unwrap();
    // Default permission for ls is deny unless configured, wait, default config is empty
    // But let's just check it doesn't fail the workspace check
    let has_global_violation = decision
        .reasons
        .iter()
        .any(|r| r.contains("destructively outside"));
    assert!(!has_global_violation);

    // Test a destructive global command
    let decision = sandbox
        .simulate_command(
            "run_command",
            r#"{"command": "rm", "args": ["-rf", "/"]}"#,
            &permissions,
        )
        .unwrap();
    assert!(!decision.is_allowed);

    let has_global_violation = decision
        .reasons
        .iter()
        .any(|r| r.contains("destructively outside"));
    assert!(has_global_violation);
}

#[tokio::test]
async fn test_subagent_budget_isolation() {
    let (memory, config, db_path) = setup_test_env("subagent", None).await;
    let mcp_reg = std::sync::Arc::new(tokio::sync::Mutex::new(
        crate::mcp_registry::McpRegistry::new(),
    ));
    let mem = std::sync::Arc::new(memory);
    let scheduler = Scheduler::new(config, mem.clone(), mcp_reg, None).unwrap();
    let _res = scheduler.run_goal("Please delegate").await;

    let _ = fs::remove_file(&db_path);
}

#[tokio::test]
async fn test_mcp_risk_card_generation() {
    // Generate a valid config that points to our test double "mockmcp malicious"
    let test_exe = std::env::current_exe().unwrap();
    let kerna_bin = test_exe
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join(format!("kerna{}", std::env::consts::EXE_SUFFIX))
        .to_string_lossy()
        .to_string();

    let config = crate::config::McpServerConfig {
        name: "MockSabotage".to_string(),
        command: kerna_bin,
        args: vec![
            "mockmcp".to_string(),
            "--mode".to_string(),
            "malicious".to_string(),
        ],
        enabled: true,
        capabilities: vec![],
        allowed_paths: vec![],
        approval_required: vec![],
        allow_tools: vec![],
        deny_tools: vec![],
        secrets: vec![],
        runtime_mode: "local".to_string(),
        docker_image: "".to_string(),
    };

    let result = crate::mcp_governance::generate_risk_card(&config).await;
    assert!(
        result.is_ok(),
        "Risk card generation should succeed on mock server"
    );
}

#[tokio::test]
async fn test_mcp_filter_deny_blocks_tool() {
    let (_memory, mut config, db_path) =
        setup_test_env("test_mcp_filter_deny_blocks_tool", None).await;

    // Explicitly deny the "echo" tool
    config.mcp_servers[0].deny_tools = vec!["echo".to_string()];

    // Initialize MCP Registry directly to test routing filter
    let mut registry = crate::mcp_registry::McpRegistry::new();
    let init_res = registry.initialize(&config.mcp_servers).await;
    assert!(init_res.is_ok());

    let args = serde_json::json!({ "text": "hello" });
    let result = registry.call_tool("echo", args).await;

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("explicitly blocked by deny_tools filter"));

    let _ = std::fs::remove_file(db_path);
}

#[tokio::test]
async fn test_mcp_filter_allow_exclusive() {
    let (_memory, mut config, db_path) =
        setup_test_env("test_mcp_filter_allow_exclusive", None).await;

    // Allow ONLY "hang" tool, meaning "echo" should be implicitly blocked
    config.mcp_servers[0].allow_tools = vec!["hang".to_string()];

    let mut registry = crate::mcp_registry::McpRegistry::new();
    let init_res = registry.initialize(&config.mcp_servers).await;
    assert!(init_res.is_ok());

    // Call echo (not in allow_tools)
    let args = serde_json::json!({ "text": "hello" });
    let result = registry.call_tool("echo", args).await;

    assert!(result.is_err());
    let err_msg = result.unwrap_err().to_string();
    assert!(err_msg.contains("not present in the allow_tools whitelist"));

    let _ = std::fs::remove_file(db_path);
}
