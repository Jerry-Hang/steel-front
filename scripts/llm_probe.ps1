$body = '{"model":"local","temperature":0.4,"max_tokens":200,"messages":[{"role":"system","content":"你是一名现代步兵营指挥官。收到战场态势 JSON 后，为每个连队下达一条命令。输出必须是严格 JSON（不要解释），格式：{"companies":[{"order":"Assault|Hold|FlankL|FlankR|Regroup","x":0,"z":0}]}。rule: order 只能取五者之一；x/z 为世界坐标目标点，须在[-270,270]内。"},{"role":"user","content":"当前战场态势：{"battle":"128v128","side":"red","map_half":270,"enemy":{"x":-110,"z":90},"companies":[{"id":0,"strength":36,"x":106,"z":-125,"contact":true,"current":"Assault"},{"id":1,"strength":36,"x":156,"z":-33,"contact":true,"current":"Assault"},{"id":2,"strength":36,"x":149,"z":64,"contact":true,"current":"Assault"}]}"}]}'
try {
  $r = Invoke-RestMethod -Uri 'http://127.0.0.1:8080/v1/chat/completions' -Method Post -ContentType 'application/json' -Body $body -TimeoutSec 60
  Write-Host $r.choices[0].message.content
} catch {
  Write-Host ("ERR: " + $_.Exception.Message)
}
