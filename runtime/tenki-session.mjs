// Trusted, bounded demo adapter. Credentials arrive once on stdin and never
// reach guest environment variables, command arguments, or evidence.
import readline from 'node:readline';
import { TenkiSandbox, stdoutText } from '@tenkicloud/sandbox';
const lines = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
let client, session;
const reply = value => process.stdout.write(JSON.stringify(value) + '\n');
try {
  for await (const line of lines) {
    const request = JSON.parse(line);
    if (!client) {
      client = new TenkiSandbox({ authToken: request.auth_token, timeoutMs: 60000, dataPlaneReadyTimeoutMs: 60000 });
      session = await client.createAndWait({ name: 'kerna-demo-session', cpuCores: 2, memoryMb: 4096,
        allowInbound: false, allowOutbound: false, maxDurationMs: 900000,
        waitTimeoutMs: 60000, timeoutMs: 60000 });
      reply({ ready: true, network: 'inbound=false,outbound=false', lifetime_ms: 900000 });
      continue;
    }
    if (request.close) break;
    try {
      const result = await session.exec('python3', { args: ['-c', request.code], timeoutMs: Math.min(request.timeout_ms || 10000, 10000) });
      const output = stdoutText(result);
      const bounded = Buffer.byteLength(output, 'utf8') <= 65536;
      reply({ status: result.exitCode === 0 && bounded ? 'completed' : 'failed',
        exit_code: result.exitCode === 0 && bounded ? 0 : 1,
        output: bounded ? output : 'Output limit exceeded',
        package: 'tenki/sandbox-v1:' + session.id, network: 'inbound=false,outbound=false' });
    } catch {
      reply({ status: 'failed', exit_code: 1, output: 'Remote execution failed', package: 'tenki/sandbox-v1', network: 'inbound=false,outbound=false' });
    }
  }
} catch {
  reply({ error: 'Tenki session could not be prepared or continued' });
  process.exitCode = 1;
} finally {
  if (session) await session.close();
  await client?.[Symbol.asyncDispose]?.();
}
