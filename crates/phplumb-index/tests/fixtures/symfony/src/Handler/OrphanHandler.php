<?php

namespace App\Handler;

use App\Message\Unhandled;
use Symfony\Component\Messenger\Attribute\AsMessageHandler;

abstract class OrphanHandler
{
    #[AsMessageHandler]
    public function onUnhandled(Unhandled $message): void
    {
    }
}
