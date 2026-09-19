import io
import json
import re
import sys

TARGETS = {
    "sites.goApplyZCode",
    "apply.targetZCode",
    "apply.resultZCodeOk",
    "apply.groupZCodeProtocol",
    "apply.zcodeApiType",
    "apply.zcodeApiTypeHint",
    "apply.zcodeApiTypeAnthropic",
    "apply.zcodeApiTypeResponses",
    "apply.zcodeApiTypeChatCompletions",
    "apply.groupZCodeModels",
    "apply.zcodeWriteAllModels",
    "apply.zcodeWriteAllModelsHint",
    "apply.zcodeFilesHint",
    "apply.removeZCodeConfirm",
    "apply.removeZCodeHint",
    "apply.removeZCodeDone",
    "settings.zcodeHome",
    "settings.zcodeHomePlaceholder",
    "settings.zcodeHomeHint",
    "mcp.targetZCode",
    "rules.targetZCode",
    "proxy.targetZCode",
    "apply.dualWarning",
    "apply.dualWarningTitle",
}

KEY = re.compile(r'^(\s*)"((?:[^"\\]|\\.)*)"\s*:\s*(.*)$')
APPLY = "--apply" in sys.argv


def paths_of(obj, pre=""):
    out = set()
    for k, v in obj.items():
        p = f"{pre}.{k}" if pre else k
        if isinstance(v, dict):
            out |= paths_of(v, p)
        else:
            out.add(p)
    return out


for path in ["src/i18n/locales/zh-CN.json", "src/i18n/locales/en-US.json"]:
    raw = io.open(path, encoding="utf-8", newline="").read()
    nl = "\r\n" if "\r\n" in raw else "\n"
    lines = raw.split(nl)
    before = paths_of(json.loads(raw))
    stack = []
    drop = []
    for i, line in enumerate(lines):
        m = KEY.match(line)
        if not m:
            continue
        ind, key, rest = len(m.group(1)), m.group(2), m.group(3)
        while stack and stack[-1][0] >= ind:
            stack.pop()
        dotted = ".".join([s[1] for s in stack] + [key])
        if rest.rstrip().endswith("{"):
            stack.append([ind, key])
        elif dotted in TARGETS:
            drop.append((i, ind, dotted))
    found = {p for _, _, p in drop}
    print(f"--- {path}: matched {len(found)} / {len(TARGETS)}")
    for missing in sorted(TARGETS - found):
        print("    MISSING:", missing)
    for i, ind, dotted in drop:
        if not lines[i].rstrip().endswith(","):
            print("    LAST-MEMBER (next sibling keeps no comma):", dotted)
    idx = {i for i, _, _ in drop}
    # deleting the final member of an object strands a comma on the previous sibling
    strip = set()
    for i, ind, dotted in drop:
        if lines[i].rstrip().endswith(","):
            continue
        j = i - 1
        while j in idx:
            j -= 1
        if j < 0 or not lines[j].rstrip().endswith(","):
            print("    !!! cannot fix comma for", dotted)
        else:
            strip.add(j)
    out = []
    for i, line in enumerate(lines):
        if i in idx:
            continue
        out.append(line.rstrip()[:-1] if i in strip else line)
    rebuilt = nl.join(out)
    after = paths_of(json.loads(rebuilt))
    lost = sorted(before - after)
    print("    lost keys:", len(lost))
    for k in lost:
        print("      ", k)
    extra = sorted(after - before)
    if extra:
        print("    !!! EXTRA keys appeared:", extra)
    stray = [p for p in after if "zcode" in p.lower() or "z_code" in p.lower()]
    print("    surviving zcode keys:", stray)
    if APPLY:
        io.open(path, "w", encoding="utf-8", newline="").write(rebuilt)
        print("    WRITTEN")
