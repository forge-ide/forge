import unittest

import text


class TextTest(unittest.TestCase):
    def test_title_capitalizes_the_first_letter(self):
        self.assertEqual(text.title("forge"), "Forge")

    def test_slugify_lowercases_and_joins_words_with_hyphens(self):
        self.assertEqual(text.slugify("Hello, World!"), "hello-world")


if __name__ == "__main__":
    unittest.main()
