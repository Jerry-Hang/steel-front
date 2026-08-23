# -*- coding: utf-8 -*-
import io
p = 'src/llm_cmd.rs'
s = io.open(p, encoding='utf-8').read()
start = s.index('fn json_escape(s: &str) -> String {')
end = s.index('\n}', start) + 2
new_fn = '''fn json_escape(s: &str) -> String {
    s.replace('\\\\', "\\\\\\\\")
        .replace('"', "\\\"")
        .replace('\\n', "\\\\n")
        .replace('\\r', "\\\\r")
        .replace('\\t', "\\\\t")
}
'''
s = s[:start] + new_fn + s[end:]
io.open(p, 'w', encoding='utf-8', newline='').write(s)
print('fixed2')
