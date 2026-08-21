import base64, json, urllib.request, sys
sys.stdout.reconfigure(encoding='utf-8')
key = 'sk-54811b5099304920963578b4755d884b'
img = open(r'D:/Rust/steel-front/screenshots/steel_front_1787309259.png', 'rb').read()
b64 = base64.b64encode(img).decode()
body = json.dumps({
    'model': 'deepseek-v4-flash-vision-exp',
    'messages': [{'role': 'user', 'content': [
        {'type': 'text', 'text': 'Describe this game screenshot: what HUD elements at corners? Is the Chinese text (storm, in the weapon name line at bottom-left) rendered clearly? Any garbled/missing/distorted characters?' },
        {'type': 'image_url', 'image_url': {'url': 'data:image/png;base64,' + b64}}
    ]}],
    'max_tokens': 800,
}).encode()
req = urllib.request.Request('https://api.deepseek.com/chat/completions', data=body,
    headers={'Authorization': 'Bearer ' + key, 'Content-Type': 'application/json'})
try:
    resp = urllib.request.urlopen(req, timeout=180)
    data = json.loads(resp.read())
    print(data['choices'][0]['message']['content'])
except Exception as e:
    print('VISION_ERR:', e)
    try:
        print(e.read().decode()[:500])
    except Exception:
        pass