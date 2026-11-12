#!/usr/bin/env python3
"""Generate release download badges from the built asset names."""

import argparse
from pathlib import Path
from urllib.parse import quote


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--repository', required=True)
    parser.add_argument('--tag', required=True)
    parser.add_argument('assets', nargs='+', type=Path)
    args = parser.parse_args()

    platforms = (
        ('Windows x64', '-x86_64-pc-windows-msvc.zip', '0078D4'),
        ('macOS Apple Silicon', '-aarch64-apple-darwin.dmg', '555555'),
        ('macOS Intel', '-x86_64-apple-darwin.dmg', '555555'),
        ('Linux x64', '-x86_64-unknown-linux-gnu.tar.gz', 'FCC624'),
    )
    print('### 选择适合你的版本\n')
    for label, suffix, color in platforms:
        matches = [asset.name for asset in args.assets if asset.name.endswith(suffix)]
        if len(matches) != 1:
            parser.error(f'expected one {label} asset, found {len(matches)}')
        badge_label = quote(label.replace('-', '--').replace('_', '__').replace(' ', '_'), safe='')
        badge = f'https://img.shields.io/badge/{badge_label}-下载-{color}'
        download = (f'https://github.com/{args.repository}/releases/download/'
                    f'{quote(args.tag, safe="")}/{quote(matches[0], safe="")}')
        print(f'[![{label} 下载]({badge})]({download})')
    print('\nmacOS 用户打开 DMG 后，将应用拖入「应用程序」文件夹即可安装。')


if __name__ == '__main__':
    main()
