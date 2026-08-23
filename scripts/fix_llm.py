# -*- coding: utf-8 -*-
import io
p = 'src/llm_cmd.rs'
s = io.open(p, encoding='utf-8').read()

# 1) 重写 json_escape（坏字符字面量修复）
start = s.index('fn json_escape(s: &str) -> String {')
end = s.index('\n}', start) + 2
new_fn = '''fn json_escape(s: &str) -> String {
    s.replace('\\\\', "\\\\\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}
'''
s = s[:start] + new_fn + s[end:]

# 2) decide_side 去 sh 参数
s = s.replace('fn decide_side(sh: &Shared, side: &SideCtx, name: &str, url: &str) {',
              'fn decide_side(side: &SideCtx, name: &str, url: &str) {')
s = s.replace('decide_side(&sh2, &sh2.red, "red", &url);',
              'decide_side(&sh2.red, "red", &url);')
s = s.replace('decide_side(&sh2, &sh2.blue, "blue", &url);',
              'decide_side(&sh2.blue, "blue", &url);')
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('fixed')
