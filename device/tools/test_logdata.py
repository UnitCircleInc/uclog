# © 2026 Unit Circle Inc.
# SPDX-License-Identifier: Apache-2.0
"""logdata.py's file names: python3 -I -m unittest discover -s device/tools"""

import unittest

from logdata import source_name


class SourceName(unittest.TestCase):
  def test_relative_names_are_kept(self):
    self.assertEqual(source_name('src/main.c'), 'src/main.c')
    self.assertEqual(source_name('uclog/src/../src/log.c'), 'uclog/src/log.c')

  def test_windows_paths_are_unixy(self):
    self.assertEqual(source_name('src\\drivers\\sensor.c'), 'src/drivers/sensor.c')
    self.assertEqual(source_name('C:\\work\\app\\src\\main.c', [('C:/work/app/', '')]), 'src/main.c')

  def test_maps_last_match_wins(self):
    maps = [('/ws/', ''), ('/ws/app/', ''), ('/ws/modules/uclog/device', 'uclog')]
    self.assertEqual(source_name('/ws/zephyr/kernel/sched.c', maps), 'zephyr/kernel/sched.c')
    self.assertEqual(source_name('/ws/app/src/main.c', maps), 'src/main.c')
    self.assertEqual(source_name('/ws/modules/uclog/device/src/log.c', maps), 'uclog/src/log.c')
    self.assertEqual(source_name('/zb/kernel/sched.c', [('/zb/', 'zephyr/')]), 'zephyr/kernel/sched.c')

  def test_a_map_matches_whole_directories(self):
    self.assertEqual(source_name('/ws/app2/main.c', [('/ws/app', 'x')]), 'main.c')

  def test_absolute_paths_left_become_the_file_name(self):
    self.assertEqual(source_name('/Users/me/project/src/main.c'), 'main.c')
    self.assertEqual(source_name('D:/a/b/c.c'), 'c.c')


if __name__ == '__main__':
  unittest.main()
