# SPDX-License-Identifier: GPL-3.0-only
# Copyright (c) 2026 LibreType contributors
"""The timing sampler must not export unrelated native diagnostics."""
import copy
import unittest
import measure_typing


class TimingTests(unittest.TestCase):
    def status(self):
        sample = dict.fromkeys(measure_typing.FIELDS, 10.0)
        sample['event_to_draw_ms'] = 60.0
        sample['sequence'] = 3
        return {'pid': 123, 'current': {'last_typing_latency': sample,
                'unrelated': 'SYNTHETIC MUST NOT EXPORT'}, 'last_chrome': {'private': True}}

    def test_extracts_only_numeric_sample(self):
        result = measure_typing.extract(self.status(), 123)
        self.assertEqual(set(result), {'sequence', *measure_typing.FIELDS})
        self.assertNotIn('SYNTHETIC', str(result))
        self.assertIsNone(measure_typing.extract({'pid': 123, 'current': {}}, 123))

    def test_rejects_other_process_unknown_payloads_and_invalid_durations(self):
        with self.assertRaises(ValueError):
            measure_typing.extract(self.status(), 124)
        changes = [('sequence', True), ('sequence', 0), ('worker_ms', float('nan')),
                   ('worker_ms', -1), ('worker_ms', '10'), ('event_to_draw_ms', 59),
                   ('text', 'SYNTHETIC MUST NOT EXPORT')]
        for name, value in changes:
            status = copy.deepcopy(self.status())
            status['current']['last_typing_latency'][name] = value
            with self.subTest(name=name, value=value), self.assertRaises(ValueError):
                measure_typing.extract(status, 123)


if __name__ == '__main__':
    unittest.main()
