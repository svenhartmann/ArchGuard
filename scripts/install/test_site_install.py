"""The Pages build shows the install one-liners only when the repository is known."""
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
RAW = 'https://raw.githubusercontent.com/owner/repo/main/scripts/install/'


def build(*extra):
    out = Path(tempfile.mkdtemp()) / 'site'
    subprocess.run([sys.executable, str(ROOT / 'site/build.py'), '--out', str(out), *extra],
                   check=True, capture_output=True)
    subprocess.run([sys.executable, str(ROOT / 'site/check.py'), str(out)], check=True, capture_output=True)
    return out


class SiteInstallTest(unittest.TestCase):
    def test_one_liners_with_repository(self):
        out = build('--repository', 'owner/repo')
        for page in ('index.html', 'docs/getting-started.html'):
            text = (out / page).read_text()
            self.assertIn(f'curl -fsSL {RAW}install.sh | sh', text, page)
            self.assertIn(f'irm {RAW}install.ps1 | iex', text, page)

    def test_source_build_without_repository(self):
        out = build()
        for page in ('index.html', 'docs/getting-started.html'):
            text = (out / page).read_text()
            self.assertNotIn('install.sh', text, page)
            self.assertIn('cargo install --path crates/archguard-cli --locked', text, page)


if __name__ == '__main__':
    unittest.main()
