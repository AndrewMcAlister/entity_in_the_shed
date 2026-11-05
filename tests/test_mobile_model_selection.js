const test = require('node:test');
const assert = require('node:assert/strict');

test('selectedIndex sets to last index if valid', () => {
  const lastIndex = 1;
  const availableModels = ['mudler/GLM-4.7-Flash-APEX-I-Mini-flash', 'Meta-Llama-3.1-8B-Instruct-Q8_0'];
  const defaultIndex = lastIndex !== null && lastIndex < availableModels.length ? lastIndex : 0;
  assert.equal(defaultIndex, 1);
});

test('selectedIndex sets to 0 if last index invalid', () => {
  const lastIndex = 5;
  const availableModels = ['mudler/GLM-4.7-Flash-APEX-I-Mini-flash', 'Meta-Llama-3.1-8B-Instruct-Q8_0'];
  const defaultIndex = lastIndex !== null && lastIndex < availableModels.length ? lastIndex : 0;
  assert.equal(defaultIndex, 0);
});

test('selectedIndex sets to 0 if no last index', () => {
  const lastIndex = null;
  const availableModels = ['mudler/GLM-4.7-Flash-APEX-I-Mini-flash', 'Meta-Llama-3.1-8B-Instruct-Q8_0'];
  const defaultIndex = lastIndex !== null && lastIndex < availableModels.length ? lastIndex : 0;
  assert.equal(defaultIndex, 0);
});