import json, os, subprocess, sys
scratch, crate, name, results_name, prefix = sys.argv[1:6]
open_numbers = [int(n) for n in sys.argv[6].split(",")]
results = f"{scratch}/{results_name}"
found = {}
for part in range(6):
    if not open_numbers:
        break
    probe = f"{crate}/target/breaks/state/{prefix}-{part}.json"
    before = len(open(results).read().splitlines()) if os.path.exists(results) else 0
    env = dict(os.environ, LIST=name, NUMBERS=",".join(map(str, open_numbers)))
    done = subprocess.run([sys.executable, f"{crate}/tools/breaks/tree_breaks.py", scratch, probe], cwd=crate, env=env, capture_output=True, text=True)
    print("part", part, done.stdout.strip()[-200:], done.stderr.strip()[-300:], flush=True)
    for line in open(results).read().splitlines()[before:]:
        one = json.loads(line)
        if one["outcome"] != "not caught":
            found[one["number"]] = (part, one["outcome"])
            if os.path.exists(f"{results}.{one['number']}.json"):
                os.replace(f"{results}.{one['number']}.json", f"{scratch}/more-{name}.{one['number']}.json")
            open_numbers.remove(one["number"])
print("found", found, "still open", open_numbers)
