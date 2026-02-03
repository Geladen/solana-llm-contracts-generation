import sys
from crystalbleu import sentence_bleu

file1 = sys.argv[1]
file2 = sys.argv[2]

with open(file1, "r") as f1, open(file2, "r") as f2:
    code1 = f1.read().split()
    code2 = f2.read().split()

score = sentence_bleu([code1], code2)
print(f"Similarity: {score:.3f}")