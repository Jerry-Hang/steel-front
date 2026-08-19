import struct
NAMES = {103:'?103',127:'OpFNegate',128:'OpIAdd',129:'OpFAdd',131:'OpFSub',133:'OpFMul',136:'OpFDiv',61:'OpLoad',62:'OpStore',65:'OpAccessChain',79:'OpVectorShuffle',80:'OpCompositeConstruct',81:'OpCompositeExtract',82:'OpCompositeInsert',83:'OpCopyObject',142:'OpVectorTimesScalar',126:'OpSNegate',59:'OpVariable',43:'OpConstant',44:'OpConstantComposite',46:'OpConstantNull',22:'OpTypeFloat',23:'OpTypeVector',32:'OpTypePointer',19:'OpTypeVoid',21:'OpTypeInt',15:'OpEntryPoint',14:'OpMemoryModel',17:'OpCapability',33:'OpTypeFunction',218:'OpLabel',219:'OpBranch',220:'OpBranchConditional',223:'OpReturn',224:'OpReturnValue'}
data = open('D:/Rust/steel-front/assets/hud.vert.spv', 'rb').read()
words = struct.unpack('<%dI' % (len(data)//4), data)
i = 5
ops = []
while i < len(words):
    w = words[i]
    opcode = w & 0xFFFF
    wc = w >> 16
    ops.append((opcode, words[i+1:i+wc]))
    i += wc
print('total ops:', len(ops))
for idx, (op, args) in enumerate(ops):
    if op == 127:
        print('FNegate at op#', idx, 'args:', args)
        # resolve what result id feeds into: find the value id (args[1] = result)
        res = args[1]
        # what is being negated: args[2]
        neg = args[2]
        # find the instruction that produced 'neg'
        for j in range(idx-1, -1, -1):
            o2, a2 = ops[j]
            if len(a2) > 1 and a2[1] == neg:
                print('   negated value from:', NAMES.get(o2, '?'+str(o2)), list(a2))
                break
        # what uses the result
        for j in range(idx+1, len(ops)):
            o2, a2 = ops[j]
            if res in list(a2[1:]):
                print('   result used by:', NAMES.get(o2, '?'+str(o2)), list(a2))
                break
        # print all instructions in the function body
        print('   function body:')
        for j in range(max(0, idx-12), min(len(ops), idx+8)):
            o2, a2 = ops[j]
            print('     ', NAMES.get(o2, '?'+str(o2)), list(a2)[:5])