import {createRequire} from 'node:module';
import test from 'node:test'; import assert from 'node:assert/strict';
const {createStore}=createRequire(import.meta.url)('./thread-drafts.js');
test('switching threads preserves exact unsent text and clearing does not resurrect sent text',()=>{
 const store=createStore(); assert.equal(store.switchThread('a','b','A <not markup>\n'),'');
 assert.equal(store.switchThread('b','a','B'),'A <not markup>\n');
 assert.equal(store.switchThread('a','b',''),'B');
 assert.equal(store.switchThread('b','a','B revised'),'');
 assert.equal(store.switchThread('a','b',''),'B revised');
});
test('new-thread draft has its own slot and same-thread refresh preserves current text',()=>{
 const store=createStore(); assert.equal(store.switchThread(null,'a','New thread'),'');
 assert.equal(store.switchThread('a','a','Editing'),'Editing');
 assert.equal(store.switchThread('a',null,'Editing'),'New thread');
 assert.equal(store.switchThread(null,'a','New revised'),'Editing');
});
