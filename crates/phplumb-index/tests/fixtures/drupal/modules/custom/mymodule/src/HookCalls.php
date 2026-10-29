<?php
namespace Drupal\mymodule;

use Drupal\Core\Extension\ModuleHandlerInterface;
use Drupal\Core\Theme\ThemeManagerInterface;

class HookCalls
{
    public function __construct(
        private ModuleHandlerInterface $moduleHandler,
        private ThemeManagerInterface $themeManager,
    ) {
    }

    public function run(array $info, array $data): void
    {
        $this->moduleHandler->/*@target:cron_impl*/invokeAll('cron');
        \Drupal::moduleHandler()->/*@target:ping_impl*/invoke('mymodule', 'ping');
        \Drupal::moduleHandler()->/*@target:gadget_ping*/invoke(hook: 'ping', module: 'gadget');
        \Drupal::moduleHandler()->/*@target:ping_impl*/invoke(module: 'mymodule', hook: 'ping');
        $this->moduleHandler->/*@target:thing_info_module*/alter('thing_info', $info);
        $this->themeManager->/*@target:thing_info_theme*/alter('thing_info', $info);
        $this->moduleHandler->/*@target:thing_data*/alter(['thing_data', 'other'], $data);
    }
}
