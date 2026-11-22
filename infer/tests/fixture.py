"""The suite's tiny models: a one-layer llama over a byte-level vocab, so any text tokenises one
token a byte and each special one token. `decider.gguf` carries the pplx_decider keys sandman-infer
reads, `embed.gguf` none. Deterministic: seeded weights, gguf and numpy pinned. Regenerate with
uv run --no-project --with gguf==0.19.0 --with numpy==2.5.5 python -I infer/tests/fixture.py infer/tests
"""

import sys
from pathlib import Path

import gguf
import numpy

SPECIALS = ["<|endoftext|>", "<|im_start|>", "<|im_end|>", "<|fim_suffix|>", "<think>", "</think>"]
EMBD, FF, HEADS = 16, 32, 2


def write(path: Path, decider: bool) -> None:
    # gpt-2's byte-to-unicode map: the printable bytes first, so "A" and "B" are 32 and 33
    printable = [*range(33, 127), *range(161, 173), *range(174, 256)]
    rest = [byte for byte in range(256) if byte not in printable]
    tokens = [chr(byte) for byte in printable] + [chr(256 + n) for n in range(len(rest))]
    # one merge, of two spaces, which no test text holds: llama.cpp wants a merges key
    tokens += ["ĠĠ", *SPECIALS]
    vocab = len(tokens)
    out = gguf.GGUFWriter(str(path), "llama")
    out.add_block_count(1)
    out.add_context_length(16_384)
    out.add_embedding_length(EMBD)
    out.add_feed_forward_length(FF)
    out.add_head_count(HEADS)
    out.add_head_count_kv(HEADS)
    out.add_layer_norm_rms_eps(1e-5)
    out.add_rope_dimension_count(EMBD // HEADS)
    out.add_file_type(gguf.LlamaFileType.ALL_F32)
    out.add_tokenizer_model("gpt2")
    out.add_tokenizer_pre("qwen2")
    out.add_token_list(tokens)
    out.add_token_types([gguf.TokenType.NORMAL] * (vocab - len(SPECIALS)) + [gguf.TokenType.CONTROL] * len(SPECIALS))
    out.add_token_merges(["Ġ Ġ"])
    out.add_eos_token_id(tokens.index("<|endoftext|>"))
    out.add_add_bos_token(False)
    if decider:
        out.add_array("pplx_decider.codes", ["A", "B", "C"])
        out.add_array("pplx_decider.escape_tokens", ["<|fim_suffix|>", "<|endoftext|>", "<|im_end|>"])
        out.add_string("pplx_decider.marker", "<|fim_suffix|>")
        out.add_float64("pplx_decider.temperature", 1.5539092221462472)
        out.add_string("pplx_decider.template", "joint-question-inline-readout-v1")
        out.add_array("pplx_decider.token_ids", [32, 33, 34])
    rng = numpy.random.default_rng(7)
    shapes = {
        "token_embd.weight": (vocab, EMBD),
        "output_norm.weight": (EMBD,),
        "output.weight": (vocab, EMBD),
        "blk.0.attn_norm.weight": (EMBD,),
        "blk.0.attn_q.weight": (EMBD, EMBD),
        "blk.0.attn_k.weight": (EMBD, EMBD),
        "blk.0.attn_v.weight": (EMBD, EMBD),
        "blk.0.attn_output.weight": (EMBD, EMBD),
        "blk.0.ffn_norm.weight": (EMBD,),
        "blk.0.ffn_gate.weight": (FF, EMBD),
        "blk.0.ffn_up.weight": (FF, EMBD),
        "blk.0.ffn_down.weight": (EMBD, FF),
    }
    for name, shape in shapes.items():
        out.add_tensor(name, rng.standard_normal(shape, dtype=numpy.float32))
    out.write_header_to_file()
    out.write_kv_data_to_file()
    out.write_tensors_to_file()
    out.close()


for name, decider in (("decider.gguf", True), ("embed.gguf", False)):
    write(Path(sys.argv[1]) / name, decider)
