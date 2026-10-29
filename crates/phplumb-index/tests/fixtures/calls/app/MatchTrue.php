<?php

namespace App;

class RunEvidence
{
    public int /*@target:arm*/$ownerId = 0;
}

class OtherEvidence
{
    public int $ownerId = 0;
}

function owner(object $message): ?int
{
    return match (true) {
        $message instanceof RunEvidence => $message->/*@cursor:arm*/ownerId,
        is_string($message) => $message->/*@cursor:other*/ownerId,
        $message instanceof RunEvidence || $message instanceof OtherEvidence => $message->/*@cursor:either*/ownerId,
        default => null,
    };
}
