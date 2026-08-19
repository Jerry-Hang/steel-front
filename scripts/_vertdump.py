import struct
NAMES = {127:'OpFNegate',61:'OpLoad',62:'OpStore',65:'OpAccessChain',79:'OpVectorShuffle',80:'OpCompositeConstruct',81:'OpCompositeExtract',82:'OpCompositeInsert',83:'OpCopyObject',142:'OpVectorTimesScalar',126:'OpSNegate',129:'OpFAdd',131:'OpFSub',133:'OpFMul',136:'OpFDiv',128:'OpIAdd',59:'OpVariable',43:'OpConstant',44:'OpConstantComposite',46:'OpConstantNull',22:'OpTypeFloat',23:'OpTypeVector',32:'OpTypePointer',19:'OpTypeVoid',21:'OpTypeInt',15:'OpEntryPoint',14:'OpMemoryModel',17:'OpCapability',33:'OpTypeFunction',218:'OpLabel',219:'OpBranch',220:'OpBranchConditional',223:'OpReturn',224:'OpReturnValue',71:'OpFunction',72:'OpFunctionParameter',73:'OpFunctionEnd',41:'OpTypePointer',33:'OpTypeFunction'}
data = open('D:/Rust/steel-front/assets/triangle.vert.spv', 'rb').read()
words = struct.unpack('<%dI' % (len(data)//4), data)
i = 5
ops = []
while i < len(words):
    w = words[i]
    opcode = w & 0xFFFF
    wc = w >> 16
    ops.append((opcode, words[i+1:i+wc]))
    i += wc
# print ops 200-253 (the tail of main)
for idx in range(195, min(len(ops), 254)):
    op, args = ops[idx]
    print(idx, NAMES.get(op, '?'+str(op)), list(args)[:6])