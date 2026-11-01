"""Summarize native process.csv + phase/heartbeat logs using steady windows."""
import csv
import datetime as dt
import json
from pathlib import Path
import re
import statistics as stats
import sys

root = Path(__file__).resolve().parent.parent

def timestamp(value):
    return dt.datetime.fromisoformat(value.replace("Z", "+00:00")).timestamp()

def percentile(values, q):
    values = sorted(values)
    return values[min(len(values)-1, int((len(values)-1)*q))]

def summarize(label):
    folder = root / "docs/verification" / ("perf-" + label)
    rows = list(csv.DictReader((folder / "process.csv").open(encoding="utf-8-sig")))
    for row in rows:
        row["time"] = timestamp(row["utc"])
    events, beats = [], []
    for line in (folder / "native.log").read_text(encoding="utf8").splitlines():
        time = timestamp(line.split()[0])
        phase = re.search(r"PERF phase=(\w+)", line)
        if phase:
            round_match = re.search(r"\bround=(\d+)", line)
            open_match = re.search(r"\bopen_ms=(\d+)", line)
            events.append({"phase":phase[1], "time":time, "round":int(round_match[1]) if round_match else -1,
                           "openMs":int(open_match[1]) if open_match else None})
        beat = re.search(r"PERF heartbeat late_ms=(\d+)",line)
        if beat:
            beats.append((time,int(beat[1])))
    result = {"label":label, "machine":json.loads((folder / "machine.json").read_text(encoding="utf-8-sig")), "phases":[]}
    for event in events:
        phase = event["phase"]
        offset, duration = (1,2) if phase == "idle" else (2,4) if phase == "closed" else (1,6) if phase in ("static50","gif10") else (1,8)
        start, end = event["time"] + offset, event["time"] + offset + duration
        sample = [r for r in rows if start <= r["time"] < end]
        if not sample:
            continue
        summary = {"phase":phase,"round":event["round"],"openMs":event["openMs"],"samples":len(sample)}
        for column in ("workingSetMiB","privateMiB","cpuTotalPercent","threads","handles"):
            values = [float(r[column]) for r in sample]
            summary[column] = {"mean":stats.mean(values),"p95":percentile(values,.95),"max":max(values)}
        times = [r["time"]-start for r in sample]
        private = [float(r["privateMiB"]) for r in sample]
        tx, ty = stats.mean(times), stats.mean(private)
        variance = sum((x-tx)**2 for x in times)
        summary["privateSlopeMiBPerSecond"] = sum((x-tx)*(y-ty) for x,y in zip(times,private))/variance if variance else 0
        late = [value for time,value in beats if start <= time < end]
        if late:
            summary["uiHeartbeatLateMs"] = {"mean":stats.mean(late),"p95":percentile(late,.95),"max":max(late)}
        result["phases"].append(summary)
    result["peakPrivateMiB"] = max(float(r["privateMiB"]) for r in rows)
    result["peakWorkingSetMiB"] = max(float(r["workingSetMiB"]) for r in rows)
    return result

results = [summarize(label) for label in (sys.argv[1:] or ["local"])]
output = root / "docs/verification/perf-comparison.json"
output.write_text(json.dumps(results,indent=2),encoding="utf8")
for result in results:
    print(result["label"], "peak private MiB:", round(result["peakPrivateMiB"],1))
    for phase in result["phases"]:
        print(phase["round"], phase["phase"], "private MiB",round(phase["privateMiB"]["mean"],1),
              "CPU %", round(phase["cpuTotalPercent"]["mean"],2),
              "memory MiB/s",round(phase["privateSlopeMiBPerSecond"],2),
              "UI late p95 ms",phase.get("uiHeartbeatLateMs",{}).get("p95"))
