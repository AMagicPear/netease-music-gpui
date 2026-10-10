"""把网易云官方 web 包里的图标组件还原成 `assets/icons/` 下的 svg 文件。

参考 open-orpheus 对官方网易云客户端资源的下载。

网易云 PC 客户端的界面是网页（代号 orpheus），open-orpheus 会把它从官方客户端里
抠出来、校验签名后缓存成 `orpheus.ntpk`：

    100 字节头 + 原样 ZIP
      0..3   版本号，ASCII 十进制，如 "0003"
      4..35  32 字节 ASCII 十六进制元数据（内容哈希）
      36..99 64 字节 RSA-512 + SHA1 签名（PKCS#1 v1.5）
    ZIP 里是网页本体（pub/hybrid/*.js 等）。

解出来只要跳过前 100 字节：

    dd if=orpheus.ntpk bs=1 skip=100 of=web.zip && unzip web.zip -d web

（macOS 上它缓存于
 `~/Library/Application Support/open-orpheus/package/package/orpheus.ntpk`。）

## 图标怎么还原

网页里每个图标都是一个 React 组件，由构建工具批量生成：

    X = hoc((function(e){return createElement("svg", Object.assign({...}, e),
            createElement("path", {d:"...", fill:"currentColor"}))}), "name", [0,0,w,h])

`assets/icons/` 下的文件就是这些调用序列化回标记语言的结果——属性名
camelCase 转 kebab-case、`!0`/`!1` 转 `true`/`false`、子节点嵌套，属性顺序保持原样。
所以还原是机械的，本脚本做的就是这件事。

用法：

    python3 scripts/extract_icons.py <chunk.js> <输出目录> name:viewbox [...]

`viewbox` 写 `0,0,w,h`，和组件尾部那个数组一致（同名图标可能有多个尺寸，
靠它区分）。例如窗口按钮：

    python3 scripts/extract_icons.py web/pub/hybrid/app.chunk.*.js /tmp/out \\
        minimize:0,0,20,20 maximize:0,0,20,20 restore:0,0,24,24 close:0,0,20,20

校验办法：拿项目里已有的图标（unfold / xpoint / collect / message …）跑一遍，
输出应当与 `assets/icons/` 下的文件逐字节相同；`setting` 这类带 clipPath 的会
只差一个生成的 id。
"""

import os
import re
import sys

# SVG 自己的 camelCase 属性名必须原样保留，不参与 camelCase -> kebab-case。
SVG_CAMEL = {
    "viewBox",
    "preserveAspectRatio",
    "gradientUnits",
    "patternUnits",
    "clipPathUnits",
    "markerWidth",
    "markerHeight",
    "refX",
    "refY",
    "textLength",
    "lengthAdjust",
    "baseProfile",
    "zoomAndPan",
}

MARKER = re.compile(r',"([A-Za-z0-9_\-]+)",\[(\d+),(\d+),(\d+),(\d+)\]\)')


def attribute_name(name):
    if name in SVG_CAMEL:
        return name
    name = name.replace("xmlnsXlink", "xmlns:xlink")
    return re.sub(r"[A-Z]", lambda m: "-" + m.group(0).lower(), name)


class Parser:
    """够用的 createElement 解析器：属性对象、字符串、!0/!1、嵌套子节点。"""

    def __init__(self, source):
        self.source = source
        self.at = 0

    def skip_space(self):
        while self.at < len(self.source) and self.source[self.at] in " \n\t":
            self.at += 1

    def take(self, token):
        self.skip_space()
        if not self.source.startswith(token, self.at):
            raise ValueError(f"expected {token!r} at {self.at}")
        self.at += len(token)

    def string(self):
        self.skip_space()
        end = self.source.index('"', self.at + 1)
        out = self.source[self.at + 1 : end]
        self.at = end + 1
        return out

    def value(self):
        self.skip_space()
        for literal, out in (("!1", "false"), ("!0", "true")):
            if self.source.startswith(literal, self.at):
                self.at += 2
                return out
        if self.source[self.at] == '"':
            return self.string()
        end = self.at
        while self.source[end] not in ",})":
            end += 1
        out = self.source[self.at : end].strip()
        self.at = end
        return out

    def attributes(self):
        self.take("{")
        out = []
        while True:
            self.skip_space()
            if self.source[self.at] == "}":
                self.at += 1
                return out
            if self.source[self.at] == ",":
                self.at += 1
                continue
            colon = self.source.index(":", self.at)
            name = self.source[self.at : colon].strip().strip('"')
            self.at = colon + 1
            out.append((attribute_name(name), self.value()))

    def node(self):
        self.take("i.createElement(")
        tag = self.string()
        self.take(",")
        self.skip_space()
        if self.source.startswith("Object.assign(", self.at):
            self.at += len("Object.assign(")
            attrs = self.attributes()
            self.take(",e)")
        elif self.source[self.at] == "{":
            attrs = self.attributes()
        else:
            self.take("null")
            attrs = []
        children = []
        while True:
            self.skip_space()
            if self.source.startswith(")", self.at):
                self.at += 1
                return tag, attrs, children
            if self.source[self.at] == ",":
                self.at += 1
                continue
            children.append(self.node())


def render(tag, attrs, children):
    attributes = "".join(f' {name}="{value}"' for name, value in attrs)
    return f"<{tag}{attributes}>{''.join(render(*child) for child in children)}</{tag}>"


def extract(source, name, viewbox):
    """取 `<name>` 且 viewBox 标记为 `viewbox`（`0,0,w,h`）的那个图标组件。"""
    end = source.index(f',"{name}",[{viewbox}])')
    start = source.rindex('i.createElement("svg"', 0, end)
    parser = Parser(source)
    parser.at = start
    return render(*parser.node())


def main(argv):
    if len(argv) < 4:
        print(__doc__)
        return 1
    chunk, output = argv[1], argv[2]
    source = open(chunk, encoding="utf-8", errors="replace").read()
    os.makedirs(output, exist_ok=True)
    for spec in argv[3:]:
        name, _, viewbox = spec.partition(":")
        if not viewbox:
            print(f"skip {spec!r}: expected name:viewbox", file=sys.stderr)
            continue
        svg = extract(source, name, viewbox)
        path = os.path.join(output, f"{name}.svg")
        with open(path, "w", encoding="utf-8") as file:
            file.write(svg)
        print(f"{path}  {len(svg)} bytes")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
