import process from "node:process";

const chunks = [];
for await (const chunk of process.stdin) chunks.push(chunk);
const request = JSON.parse(Buffer.concat(chunks).toString("utf8"));

async function runWasmer() {
  const { Wasmer } = await import("@wasmer/sdk/node");
  const wasmer = new Wasmer({ cacheRoot: process.env.KERNA_WASMER_CACHE });
  let sandbox;
  try {
    sandbox = await wasmer.sandboxes.create({
      packages: ["python/python@=3.13.18"],
      files: { "main.py": request.code },
    });
    try {
      const output = await sandbox
        .command("python", ["/workspace/main.py"])
        .run({ timeout: request.timeout_ms });
      return {
        status: output.ok ? "completed" : "failed",
        exit_code: output.ok ? 0 : 1,
        output: output.text().slice(0, 65536),
        package: "python/python@=3.13.18",
        network: "disabled",
      };
    } catch (error) {
      return {
        status: "failed",
        exit_code: 1,
        output: String(error?.message || error).slice(0, 65536),
        package: "python/python@=3.13.18",
        network: "disabled",
      };
    }
  } finally {
    if (sandbox) await sandbox.close();
    await wasmer.close();
  }
}

async function runTenki() {
  const { TenkiSandbox, stdoutText } = await import("@tenkicloud/sandbox");
  const client = new TenkiSandbox({ authToken: request.auth_token });
  let session;
  try {
    session = await client.createAndWait({
      name: "kerna-governed-execution",
      cpuCores: 2,
      memoryMb: 4096,
      allowInbound: false,
      allowOutbound: false,
    });
    const result = await session.exec("python3", {
      args: ["-c", request.code],
      timeoutMs: request.timeout_ms,
    });
    return {
      status: result.exitCode === 0 ? "completed" : "failed",
      exit_code: result.exitCode,
      output: stdoutText(result).slice(0, 65536),
      package: "tenki/sandbox-v1",
      network: "inbound=false,outbound=false",
    };
  } finally {
    if (session) await session.close();
  }
}

try {
  const result = request.backend === "tenki" ? await runTenki() : await runWasmer();
  process.stdout.write(JSON.stringify(result));
} catch (error) {
  process.stderr.write(String(error?.message || error));
  process.exitCode = 1;
}
