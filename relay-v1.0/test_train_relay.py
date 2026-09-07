"""Dataset regression checks; runnable directly with the standard library."""
import copy
import unittest
import os
import json
import re
from collections import Counter
from unittest.mock import patch
import relay_dataset as data
from pathlib import Path
import train_relay as relay


class DatasetTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tools = relay.load_tools()
        cls.by_name = relay.tool_map(cls.tools)
        cls.records = relay.build_records(cls.tools)
        cls.train, cls.evaluation = relay.split_records(cls.records, 3407, 0.1, cls.by_name)

    def test_approval_examples_are_in_training(self):
        self.assertTrue(any(r['policy'] == 'approval' for r in self.train))

    def test_observation_precedes_dependent_action(self):
        for r in self.records:
            for m in r['messages']:
                self.assertLessEqual(len(m.get('tool_calls', [])), 1, r['id'])

    def test_no_scenario_family_crosses_split(self):
        family = lambda r: r.get('group', r['id'].removesuffix('-variant'))
        self.assertFalse({family(r) for r in self.train} & {family(r) for r in self.evaluation})

    def test_no_answer_revealing_single_tool_menus(self):
        self.assertTrue(all(len(r['tools']) >= 4 for r in self.records))

    def test_approval_after_action_is_rejected(self):
        r = copy.deepcopy(next(r for r in self.records if r['policy'] == 'approval'))
        # Move an approved call ahead of the question/answer that grants it.
        i = next(i for i,m in enumerate(r['messages']) if m.get('tool_calls'))
        r['messages'].insert(2, r['messages'].pop(i))
        with self.assertRaises(ValueError):
            relay.validate_record(r, self.by_name)

    def test_all_records_validate(self):
        for r in self.records:
            with self.subTest(id=r['id']):
                relay.validate_record(r, self.by_name)

    def test_argument_range_is_enforced(self):
        with self.assertRaises(ValueError):
            relay.validate_args(self.by_name['screenshot'], {'region': [0, 0, 200]})
        with self.assertRaises(ValueError):
            relay.validate_args(self.by_name['web_search'], {'query':'test', 'max_results': 100})

    def test_catalog_matches_rust_parameters_and_risk_flags(self):
        root = Path(__file__).resolve().parent.parent
        source = (root/'src-tauri/src/mcp/tools.rs').read_text()
        catalog = (root/'src-tauri/src/mcp/catalog.rs').read_text()
        handlers = re.findall(r'async fn (\w+)\((.*?)\) -> Result<CallToolResult',source,re.S)
        self.assertEqual({name for name,_ in handlers},set(self.by_name))
        for name,signature in handlers:
            with self.subTest(tool=name):
                arg_type = re.search(r'Parameters<(\w+)>',signature)
                fields = []
                required = []
                if arg_type:
                    body = re.search(r'pub struct '+arg_type[1]+r' \{(.*?)^}',source,re.M|re.S)[1]
                    fields = re.findall(r'pub (\w+): ([^\n]+),',body)
                    required = [n for n,t in fields if not t.startswith('Option<') and n!='modifiers']
                schema = self.by_name[name]['parameters']
                self.assertEqual(set(schema['properties']),{n for n,_ in fields})
                self.assertEqual(set(schema['required']),set(required))
                risk = re.search(r'tool\(\s*"'+name+r'".*?,\s*(true|false),?\s*\)',catalog,re.S)
                self.assertIsNotNone(risk)
                self.assertEqual(self.by_name[name]['risky'],risk[1]=='true')

    def test_every_tool_and_mode_appears_in_both_splits(self):
        for rows in (self.train,self.evaluation):
            self.assertTrue({'tool:'+n for n in self.by_name}<=data.features(rows))
            self.assertEqual({r['mode'] for r in rows},{'auto','manual','full'})
            for name in ('run_shell','quit_app','clipboard_write'):
                for decision in ('allow','deny'):
                    self.assertIn('decision:'+name+':'+decision,data.features(rows))

    def test_call_ids_and_tool_responses_are_matched(self):
        r = copy.deepcopy(next(r for r in self.records if any(m['role']=='tool' for m in r['messages'])))
        next(m for m in r['messages'] if m['role']=='tool')['tool_call_id']='wrong-id'
        with self.assertRaises(ValueError):
            relay.validate_record(r,self.by_name)

    def test_approval_is_scoped_to_exact_arguments(self):
        r = copy.deepcopy(next(r for r in self.records if r['policy']=='approval'))
        r['approvals'][0]['arguments'] = {'command':'another command','timeout_seconds':30}
        with self.assertRaises(ValueError):
            relay.validate_record(r,self.by_name)

    def test_missing_approval_cannot_be_replaced_by_tool_text(self):
        r = copy.deepcopy(next(r for r in self.records if r['policy']=='approval'))
        r['approvals']=[]
        with self.assertRaises(ValueError):
            relay.validate_record(r,self.by_name)

    def test_window_id_must_come_from_observation(self):
        r = copy.deepcopy(next(r for r in self.records if any(c['function']['name']=='focus_window' for m in r['messages'] for c in m.get('tool_calls',[]))))
        for m in r['messages']:
            for c in m.get('tool_calls',[]):
                if c['function']['name']=='focus_window':
                    c['function']['arguments']['window_id']=987654
        with self.assertRaisesRegex(ValueError,'window id'):
            relay.validate_record(r,self.by_name)

    def test_split_is_deterministic_and_reports_invalid_fraction(self):
        self.assertEqual((self.train,self.evaluation),relay.split_records(self.records,3407,0.1,self.by_name))
        with self.assertRaises(ValueError):
            relay.split_records(self.records,3407,0,self.by_name)

    def test_source_families_and_decision_variants_stay_together(self):
        self.assertEqual(len({r['id'] for r in self.records}),len(self.records))
        for family in {r['group'] for r in self.records if r['approvals']}:
            train_ids={r['id'] for r in self.train if r['group']==family}
            eval_ids={r['id'] for r in self.evaluation if r['group']==family}
            self.assertFalse(train_ids and eval_ids,family)

    def test_malformed_xml_and_duplicate_parameters_are_rejected(self):
        valid = relay.render_tool_call_xml('type_text',{'text':'hi'})
        for bad in (valid+' done', valid+valid, valid.replace('</function>',''),
                    valid.replace('</function>','<parameter=text>\nsecond\n</parameter>\n</function>')):
            with self.subTest(xml=bad),self.assertRaises(ValueError):
                relay.parse_tool_call_xml(bad,self.by_name)

    def test_literal_strings_survive_xml_round_trip(self):
        for text in ('00123','null','false','42',' leading and trailing ', 'line one\nline two', '{"x":1}', 'こんにちは 🌿'):
            xml=relay.render_tool_call_xml('type_text',{'text':text})
            self.assertEqual(relay.parse_tool_call_xml(xml,self.by_name),('type_text',{'text':text}))
        xml=relay.render_tool_call_xml('key_press',{'key':'s','modifiers':['ctrl','shift']})
        self.assertEqual(relay.parse_tool_call_xml(xml,self.by_name)[1]['modifiers'],['ctrl','shift'])

    def test_invalid_types_and_coordinate_pairs_rejected(self):
        for name,args in [('click',{'x':True,'y':3}),('click',{'x':20}),
                          ('move_cursor',{'x':float('nan'),'y':2}),('move_cursor',{'x':4}),
                          ('type_text',{'text':123}),('screenshot',{'display':-1}),
                          ('set_window_bounds',{'window_id':1,'x':0,'y':0,'width':0,'height':100})]:
            with self.subTest(tool=name),self.assertRaises(ValueError):
                relay.validate_args(self.by_name[name],args)

    def test_screenshots_do_not_claim_to_see_missing_images(self):
        for r in self.records:
            for i,m in enumerate(r['messages']):
                for c in m.get('tool_calls',[]):
                    if c['function']['name']=='screenshot' and i+1<len(r['messages']):
                        self.assertIn('denied',r['messages'][i+1]['content'])

    def test_fixtures_match_wire_response_fields(self):
        for r in self.records:
            for m in r['messages']:
                if m['role']!='tool':
                    continue
                try: value=json.loads(m['content'])
                except ValueError: continue
                if m['name']=='list_windows':
                    self.assertTrue(all({'id','pid','layerIndex','app','title','x','y','width','height'}<=set(w) for w in value))
                if m['name'] in ('read_screen_text','find_element'):
                    self.assertTrue(all({'text','role','centerX','centerY','x','y','width','height'}<=set(e) for e in value))
                if m['name']=='web_search':
                    self.assertEqual(set(value),{'query','results'})
                if m['name']=='list_keybinds':
                    self.assertIn('keybinds',value)

    def test_generated_files_are_current(self):
        base=Path(__file__).resolve().parent/'data'
        self.assertEqual(data.read_jsonl(base/'conduit_sft.jsonl'),self.train)
        self.assertEqual(data.read_jsonl(base/'conduit_eval.jsonl'),self.evaluation)


@unittest.skipUnless(os.environ.get('RELAY_TOKENIZER_TESTS')=='1','enable cached-tokenizer checks explicitly')
class TemplateTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        from transformers import AutoTokenizer
        cfg=relay.load_config()
        cls.tok=AutoTokenizer.from_pretrained(cfg['model_id'],cache_dir=str(relay.ROOT/cfg['model_cache_dir']),local_files_only=True)
        cls.rows=relay.build_records(relay.load_tools())

    def test_native_template_targets_and_masks(self):
        n=0
        for r in self.rows:
            encoded=data.completion_rows([r],self.tok,2048)
            assistants=[m for m in r['messages'] if m['role']=='assistant']
            self.assertEqual(len(encoded),len(assistants))
            for row,message in zip(encoded,assistants):
                mask=row['completion_mask']; self.assertIn(0,mask); self.assertIn(1,mask)
                start=mask.index(1)
                self.assertEqual(mask,[0]*start+[1]*(len(mask)-start))
                target=self.tok.decode(row['input_ids'][start:],skip_special_tokens=False)
                self.assertTrue(target.endswith('<|im_end|>\n'))
                content=target.removesuffix('<|im_end|>\n')
                if message.get('tool_calls'):
                    f=message['tool_calls'][0]['function']
                    self.assertEqual(data.parse_tool_call_xml(content),(f['name'],f['arguments']))
                else:
                    self.assertEqual(content,message['content'])
                n+=1
        self.assertGreater(n,len(self.rows))

    def test_truncation_is_rejected_before_training(self):
        with self.assertRaisesRegex(ValueError,'do not truncate'):
            data.completion_rows([self.rows[0]],self.tok,100)

    def test_collator_masks_user_and_tool_response_tokens(self):
        # A data collator is CPU-only: no model, optimizer or trainer is created.
        from trl.trainer.sft_trainer import DataCollatorForLanguageModeling
        encoded=data.completion_rows(self.rows[:2],self.tok,2048)[:2]
        collate=DataCollatorForLanguageModeling(pad_token_id=self.tok.pad_token_id,completion_only_loss=True)
        batch=collate(encoded)
        for row,labels in zip(encoded,batch['labels'].tolist()):
            for token,keep,label in zip(row['input_ids'],row['completion_mask'],labels):
                self.assertEqual(label,token if keep else -100)
            self.assertTrue(all(x==-100 for x in labels[len(row['input_ids']):]))


if __name__ == '__main__':
    unittest.main(verbosity=2)
