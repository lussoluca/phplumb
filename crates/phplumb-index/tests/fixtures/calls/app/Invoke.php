<?php

namespace App;

class Track
{
    public function /*@target:invoke*/toString(): string
    {
        return '';
    }
}

class SaveTrack
{
    public function __invoke(int $id): Track
    {
        return new Track();
    }
}

class Plain
{
}

class Recorder
{
    public function __construct(private SaveTrack $saveTrack, private Plain $plain)
    {
    }

    public function run(): void
    {
        ($this->saveTrack)(1)->/*@cursor:invoke*/toString();
        ($this->plain)(1)->/*@cursor:plain*/toString();
    }
}
