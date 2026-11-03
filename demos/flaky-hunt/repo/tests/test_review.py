import unittest

from shipit.review import pick_reviewer


class ReviewTest(unittest.TestCase):
    def test_nobody_reviews_their_own_change(self):
        team = ["ana", "bo", "cy", "dee"]
        self.assertNotEqual(pick_reviewer("ana", team), "ana")
