"""Build the dependency-free GitHub Pages site with shared product assets."""
from pathlib import Path
import shutil

root = Path(__file__).resolve().parent.parent
target = root / "dist" / "site"
target.mkdir(parents=True, exist_ok=True)
for name in ("index.html", "style.css", "app.js"):
    shutil.copy2(root / "site" / name, target / name)
for folder in ("icons", "screenshots"):
    shutil.copytree(root / "assets" / folder, target / "assets" / folder, dirs_exist_ok=True)
(target / ".nojekyll").touch()
(target / "robots.txt").write_text(
    "User-agent: *\nAllow: /\nSitemap: https://junyi-deep.github.io/simple-snip/sitemap.xml\n",
    encoding="utf-8",
)
(target / "sitemap.xml").write_text(
    '<?xml version="1.0" encoding="UTF-8"?>\n'
    '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">'
    '<url><loc>https://junyi-deep.github.io/simple-snip/</loc></url></urlset>\n',
    encoding="utf-8",
)
print(f"GitHub Pages artifact ready: {target}")
