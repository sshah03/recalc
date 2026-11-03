import unittest

from shipit.tags import render


class TagsTest(unittest.TestCase):
    def test_labels_render_in_a_stable_order(self):
        self.assertEqual(render(["db", "api", "db"]), "api, db")
