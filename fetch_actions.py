import urllib.request
import json

req = urllib.request.Request('https://api.github.com/repos/premxai/kerna/actions/runs', headers={'User-Agent': 'Mozilla/5.0'})
res = urllib.request.urlopen(req)
runs = json.loads(res.read())['workflow_runs']
latest = runs[0]

job_req = urllib.request.Request(latest['jobs_url'], headers={'User-Agent': 'Mozilla/5.0'})
job_res = urllib.request.urlopen(job_req)
jobs = json.loads(job_res.read())['jobs']

for j in jobs:
    if j['conclusion'] == 'failure':
        print(f"Job {j['name']} failed!")
        log_req = urllib.request.Request(f"https://api.github.com/repos/premxai/kerna/actions/jobs/{j['id']}/logs", headers={'User-Agent': 'Mozilla/5.0'})
        try:
            log_res = urllib.request.urlopen(log_req)
            log = log_res.read().decode('utf-8')
            print("--- LOG ---")
            lines = log.split('\n')
            for line in lines[-100:]:
                if "error" in line.lower() or "vulnerabilit" in line.lower() or "cve" in line.lower() or "RUSTSEC" in line:
                    print(line)
        except Exception as e:
            print("Failed to download logs:", e)
        break
