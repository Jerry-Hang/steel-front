import struct
OPNAMES = {2:'TypeVoid',3:'TypeBool',4:'TypeInt',5:'TypeFloat',21:'TypeVector',33:'TypePointer',36:'TypeFunction',54:'TypeStruct',59:'Constant',61:'ConstantComposite',65:'ConstantNull',71:'Variable',77:'Load',78:'Store',126:'FNegate',127:'OpFNegate',59:'Constant',247:'CompositeExtract',248:'CompositeInsert',61:'ConstantComposite',65:'ConstantNull',71:'Variable',248:'CompositeInsert'}
def scan(path):
    data = open(path, 'rb').read()
    words = struct.unpack('<%dI' % (len(data)//4), data)
    i = 5
    ops = []
    while i < len(words):
        w = words[i]
        opcode = w & 0xFFFF
        wc = w >> 16
        ops.append((i, opcode, words[i+1:i+wc]))
        i += wc
    # find FNegate positions and print nearby instructions
    for idx, (pos, op, args) in enumerate(ops):
        if op == 127:
            print(path, 'FNegate at word', pos, 'args:', args)
            # print the load before (the thing being negated) and store after
            for j in range(max(0, idx-4), min(len(ops), idx+4)):
                p, o, a = ops[j]
                name = OPNAMES.get(o, str(o))
                print('   ', j-idx, name, a[:4])
scan('D:/Rust/steel-front/assets/mesh.spv')