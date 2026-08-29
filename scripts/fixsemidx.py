# -*- coding: utf-8 -*-
import io
s = io.open('scripts/semcheck.py', encoding='utf-8').read()
s = s.replace("    if op == 4473:  # InitializeKHR",
"""    if op == 4473:  # InitializeKHR (words: [0]=ins [1]=rq [2]=accel [3]=flags [4]=mask [5]=origin [6]=tmin [7]=dir [8]=tmax)""")
s = s.replace("        chk('Init.RayQuery', w[3], 'ptr')", "        chk('Init.RayQuery', w[1], 'ptr')")
s = s.replace("        chk('Init.Accel', w[4], 'accel')", "        chk('Init.Accel', w[2], 'accel')")
s = s.replace("        chk('Init.Flags', w[5], 'int')", "        chk('Init.Flags', w[3], 'int')")
s = s.replace("        chk('Init.Mask', w[6], 'int')", "        chk('Init.Mask', w[4], 'int')")
s = s.replace("        chk('Init.Origin', w[7], 'vec')", "        chk('Init.Origin', w[5], 'vec')")
s = s.replace("        chk('Init.TMin', w[8], 'float')", "        chk('Init.TMin', w[6], 'float')")
s = s.replace("        chk('Init.Dir', w[9], 'vec')", "        chk('Init.Dir', w[7], 'vec')")
s = s.replace("        chk('Init.TMax', w[10], 'float')", "        chk('Init.TMax', w[8], 'float')")
s = s.replace("        chk('Proceed.RayQuery', w[3], 'ptr')", "        chk('Proceed.RayQuery', w[1], 'ptr')")
s = s.replace("        if types.get(w[1]) and types[w[1]][0] != 'bool':", "        if types.get(w[1]) and types[w[1]][0] != 'bool':")
s = s.replace("        chk('GetType.RayQuery', w[3], 'ptr')", "        chk('GetType.RayQuery', w[3], 'ptr')")
s = s.replace("        chk('GetType.Intersection', w[4], 'int')", "        chk('GetType.Intersection', w[4], 'int')")
io.open('scripts/semcheck.py', 'w', encoding='utf-8', newline='').write(s)
print('indices fixed')
