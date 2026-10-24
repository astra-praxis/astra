"""校验 Mascot.tsx 里所有 IMG 路径对应的文件真实存在，并列出未使用素材。

用法: python3 design/mascot/scripts/mascot_check_assets.py
建议在新增姿态 / 重跑切图之后跑一次。
"""
import re
import sys
from pathlib import Path

# 本文件位于 design/mascot/scripts/，向上三级即仓库根
REPO_ROOT = Path(__file__).resolve().parents[3]
TSX = REPO_ROOT / "src" / "src" / "components" / "agent" / "Mascot.tsx"
PUBLIC = REPO_ROOT / "src" / "public"

tsx = TSX.read_text(encoding="utf-8")
paths = re.findall(r"'(/mascot/actions/[^']+)'", tsx)
files = {p: PUBLIC / p.lstrip("/") for p in paths}
missing = [p for p, f in files.items() if not f.exists()]

print(f"Mascot.tsx 引用 {len(set(paths))} 个图片路径")
print(f"缺失: {missing if missing else '无 ✓'}")

# 顺带统计 public/mascot/actions 下实际有多少张
actual = sorted((PUBLIC / "mascot" / "actions").glob("*.png"))
print(f"actions 目录实际 {len(actual)} 张图")

# 被引用但目录里没有的（说明路径写错）
used = {Path(p).name for p in paths}
have = {f.name for f in actual}
unused = sorted(have - used)
print(f"引用了但目录不存在: {sorted(used - have) or '无 ✓'}")
print(f"目录有但没被引用 ({len(unused)}): {unused or '无（全用上了）'}")

# 缺失即失败，便于以后接入 CI / pre-commit
sys.exit(1 if missing or (used - have) else 0)
