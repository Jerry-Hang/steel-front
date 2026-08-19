import struct
NAMES = {103:'OpFNegate',104:'OpIAdd',105:'OpFAdd',106:'OpISub',107:'OpFSub',108:'OpIMul',109:'OpFMul',112:'OpFDiv',124:'OpDot',126:'OpUMulExtended',127:'OpSMulExtended',140:'OpOrdered',218:'OpLabel',219:'OpBranch',220:'OpBranchConditional',223:'OpReturn',224:'OpReturnValue',71:'OpVariable',43:'OpLoad',44:'OpStore',47:'OpAccessChain',54:'OpDecorate',55:'OpMemberDecorate',59:'OpConstant',61:'OpConstantComposite',65:'OpConstantNull',26:'OpConstantComposite',41:'OpVariable',61:'OpConstantComposite',65:'OpConstantNull'}
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
# print the LAST 40 instructions (the main function body)
for op, args in ops[-45:]:
    print(NAMES.get(op, 'op'+str(op)), list(args)[:6])