const xs = [-2.5, -1.5, -0.5, -0.4, 0.4, 0.5, 1.5, 2.5, 2.4, 2.6, -2.6];
for (const x of xs) console.log(`round ${x} js=${Math.round(x)}`);
console.log(`fmt ${(2.5).toFixed(0)} ${(0.125).toFixed(2)} ${(0.5).toFixed(0)} ${(1.5).toFixed(0)} ${(0.375).toFixed(2)}`);
console.log(`fmt1e21 ${String(1e21)}`);
console.log(`utf8len n/a utf16 ${"😀".length} codepoints ${[..."😀"].length}`);
console.log(`sort ${JSON.stringify([10, 9, 1].sort())}`);
console.log(`json ${JSON.stringify({ b: 1, 2: 1, a: 1, 1: 1 })} nan=${JSON.stringify(NaN)} negzero=${JSON.stringify(-0)}`);
console.log(`2^53+1 ${2 ** 53 + 1} (2**31)|0=${(2 ** 31) | 0}`);
console.log(`date ${new Date("2024-01-01").toISOString()}`);
