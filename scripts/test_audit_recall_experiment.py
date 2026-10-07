"""Regression checks for complete source citation enforcement."""
import unittest
from audit_recall_experiment import validate_citation


class CitationTests(unittest.TestCase):
    def setUp(self):
        keys = ('chunkId', 'kind', 'source', 'scope', 'threadId', 'messageId',
                'noteId', 'start', 'end', 'messageSha256', 'excerptSha256')
        self.source = {key: key for key in keys}

    def test_complete(self):
        validate_citation(dict(self.source), self.source)

    def test_every_missing_field_rejected(self):
        for key in self.source:
            with self.subTest(key=key):
                altered = dict(self.source)
                del altered[key]
                with self.assertRaises(ValueError):
                    validate_citation(altered, self.source)
        with self.assertRaises(ValueError):
            validate_citation({}, self.source)

    def test_changed_and_added_fields_rejected(self):
        altered = dict(self.source, start=3)
        with self.assertRaises(ValueError):
            validate_citation(altered, self.source)
        with self.assertRaises(ValueError):
            validate_citation(dict(self.source, invented='field'), self.source)


if __name__ == '__main__':
    unittest.main()
